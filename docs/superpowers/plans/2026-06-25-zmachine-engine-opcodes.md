# Z-Machine Engine Opcodes (Core + Graceful) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a batch of standard Z-machine opcodes (currently missing or stubbed) in the `zvm` crate, raising spec coverage.

**Architecture:** Each opcode is a `match` arm in the relevant dispatch (`exec_2op`/`exec_0op`/`exec_var`/`exec_ext` in `crates/zvm/src/cpu/exec.rs`), reusing existing helpers (`do_store`, `do_branch`, `print_text`, `dictionary::tokenise`, `text::encode::encode_word`) and the `Memory` read/write API. The custom-alphabet and terminating-chars features extend `text/decode.rs` and the `read` path. All tasks are in `zvm`; they flow to `zvm-cli` automatically.

**Tech Stack:** Rust; the `zvm` crate test harness (`sample_story`, `Machine::new`, `run_until_quit`, `m.global`, `m.mem`).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-25-zmachine-engine-opcodes-design.md`.
- Commit trailers on EVERY commit body (no backticks anywhere in commit bodies — zsh):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`
- Run `cargo test -p zvm` after each task: 0 failures, 0 warnings. Treat any warning as a failure to fix.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Only the listed opcodes — do not touch `input_stream` (VAR:0x14), v5 save-to-memory, `sound_effect`, or v6.
- All arms live in `crates/zvm/src/cpu/exec.rs` unless a task says otherwise.
- Dispatch signatures: `exec_2op(opcode, ops, store, branch)`; `exec_0op(opcode, store, branch, text)`; `exec_var(opcode, ops, store, branch)`; `exec_ext(opcode, ops, store)` (store only — NO branch). `do_store(Option<u8>, u16)`, `do_branch(Option<Branch>, bool)`, `print_text(&str)`.
- Test encoding reference (existing): EXT = `0xBE op typebyte <operands> [store]`; the `log_shift_left_and_right` test is the EXT template; `je_branch_taken_and_not_taken` is the branch template. Place bytes at `0x10`, set `m.state.pc = 0x10`, `run_until_quit(&mut m)`.

---

### Task 1: verify (real checksum) + piracy (document)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — `exec_0op` arms `0x0D` (~603) and `0x0F` (~608); add a `story_checksum` helper.

**Interfaces:**
- Produces: `fn story_checksum(&self) -> u16` on `Machine` (or a free fn taking `&Memory`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn verify_branches_true_on_correct_checksum() {
    // sample_story sets a valid header checksum; verify should branch true.
    let mut buf = sample_story(5);
    // 0x10: verify (0OP:0x0D, short form 0OP = 0xB0 | opcode = 0xBD), branch on_true offset to skip an add.
    buf[0x10] = 0xBD;            // 0OP:0x0D verify
    buf[0x11] = 0xC6;            // branch: on_true=1 (bit7), short (bit6), offset=6 → skip the 4-byte add
    buf[0x12] = 0x21; buf[0x13] = 0x00; buf[0x14] = 0x07; buf[0x15] = 0x10; // add 0,7 -> G0 (skipped if branch taken)
    buf[0x16] = 0xBA;           // quit (0OP:0x0A)
    // fix the header checksum to match the story bytes (see Step 3 helper) — for the test,
    // compute it the same way story_checksum does and write it to 0x1C..0x1E.
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    let ck = m.story_checksum();
    m.mem.write_word(0x1C, ck); // header checksum = computed → verify must branch true
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert_eq!(m.global(0), 0, "verify branched true (skipped the add)");
}

#[test]
fn verify_branches_false_on_bad_checksum() {
    let mut buf = sample_story(5);
    buf[0x10] = 0xBD; buf[0x11] = 0xC6;
    buf[0x12] = 0x21; buf[0x13] = 0x00; buf[0x14] = 0x07; buf[0x15] = 0x10;
    buf[0x16] = 0xBA;
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.mem.write_word(0x1C, 0x0001); // deliberately wrong checksum
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert_eq!(m.global(0), 7, "verify branched false (ran the add)");
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p zvm verify_branches`
Expected: FAIL (`story_checksum` missing / current stub always branches true so the false test fails).

- [ ] **Step 3: Implement `story_checksum` + the arms**

Add to `impl Machine` (near `do_branch`):

```rust
/// Sum (mod 0x10000) of bytes [0x40, 0x40 + file_length). file_length =
/// header word 0x1A * scale (2 for v3, 4 for v4-5, 8 for v6-7).
pub fn story_checksum(&self) -> u16 {
    let scale: u32 = match self.mem.version() {
        1..=3 => 2,
        4 | 5 => 4,
        _ => 8,
    };
    let len = self.mem.read_word(0x1A) as u32 * scale;
    let end = (0x40 + len).min(self.mem.len() as u32);
    let mut sum: u16 = 0;
    for addr in 0x40..end {
        sum = sum.wrapping_add(self.mem.read_byte(addr) as u16);
    }
    sum
}
```

(If `Memory` has no `len()`, use the appropriate existing length accessor — grep `impl Memory` for a byte-length method.)

Replace the `0x0D` and `0x0F` arms in `exec_0op`:

```rust
// 0OP:0x0D verify — checksum the story and branch on match.
0x0D => {
    let header_ck = self.mem.read_word(0x1C);
    // If the header records no checksum (some dev builds), treat as genuine.
    let ok = header_ck == 0 || self.story_checksum() == header_ck;
    self.do_branch(branch, ok);
    StepResult::Continue
}
// 0OP:0x0F piracy — the standard says interpreters should behave as if the game
// is genuine: always take the branch.
0x0F => {
    self.do_branch(branch, true);
    StepResult::Continue
}
```

- [ ] **Step 4: Run the tests + full suite**

Run: `cargo test -p zvm`
Expected: PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): verify checksums the story (real branch); document piracy

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: set_font + set_colour + set_true_colour (graceful)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — `exec_ext` `0x04` (~1070, currently stores 0) and a new `0x05` arm; `exec_2op` new `0x1B` arm.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn set_font_reports_current_or_unavailable() {
    // EXT:0x04 set_font font -> (store). font 1 (or 0=query) -> 1; other -> 0.
    let mut buf = sample_story(5);
    // set_font 1 -> G0
    buf[0x10]=0xBE; buf[0x11]=0x04; buf[0x12]=0x7F; buf[0x13]=1; buf[0x14]=0x10; // [Small=1], store G0
    // set_font 4 -> G1
    buf[0x15]=0xBE; buf[0x16]=0x04; buf[0x17]=0x7F; buf[0x18]=4; buf[0x19]=0x11; // [Small=4], store G1
    buf[0x1A]=0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert_eq!(m.global(0), 1, "font 1 available -> 1");
    assert_eq!(m.global(1), 0, "font 4 unavailable -> 0");
}

#[test]
fn set_colour_and_true_colour_are_graceful_noops() {
    // Neither stores nor branches; just must not warn/crash and must Continue.
    let mut buf = sample_story(5);
    // set_colour 2,3 (2OP:0x1B long form, both small): opcode byte 0x1B, ops 2,3
    buf[0x10]=0x1B; buf[0x11]=2; buf[0x12]=3;
    // set_true_colour 0,0 (EXT:0x05, [Small,Small])
    buf[0x13]=0xBE; buf[0x14]=0x05; buf[0x15]=0x5F; buf[0x16]=0; buf[0x17]=0;
    buf[0x18]=0x21; buf[0x19]=0x00; buf[0x1A]=0x05; buf[0x1B]=0x10; // add 0,5 -> G0 (proves execution continued)
    buf[0x1C]=0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert_eq!(m.global(0), 5, "execution continued past set_colour/set_true_colour");
    assert!(m.diagnostics.iter().all(|d| !d.contains("0x1B") && !d.contains("0x05")),
        "graceful arms must not emit unimplemented diagnostics");
}
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p zvm set_font_reports set_colour_and_true_colour` → set_font stores 0 for font 1 (fails); the colour test may pass already (silent fall-through) but is kept as a guard.

- [ ] **Step 3: Implement the arms**

`exec_ext` — replace the `0x04` stub and add `0x05`:

```rust
// EXT:0x04 set_font — one fixed font (id 1). font 1 or 0(query) -> previous (1);
// any other requested font is unavailable -> 0. No actual font change.
0x04 => {
    let requested = ops.first().copied().unwrap_or(0);
    let result = if requested == 0 || requested == 1 { 1 } else { 0 };
    self.do_store(store, result);
    StepResult::Continue
}
// EXT:0x05 set_true_colour — we render with our own styling; accept and ignore.
0x05 => StepResult::Continue,
```

`exec_2op` — add before the `_ => Continue` fall-through:

```rust
// 2OP:0x1B set_colour — game-driven colour is not applied (lanthorn styling
// owns the look); accept and ignore.
0x1B => StepResult::Continue,
```

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): graceful set_font (report fixed font) + set_colour/set_true_colour no-ops

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 3: print_unicode + check_unicode (EXT:0x0B / 0x0C)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — new `exec_ext` arms `0x0B`, `0x0C`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn print_unicode_outputs_codepoint() {
    let mut buf = sample_story(5);
    // print_unicode 0x00E9 ('é'): EXT:0x0B, [Large operand 0x00E9]
    // type byte 0b00_11_11_11 = 0x3F ([Large, omit, omit, omit]); large = 2 bytes.
    buf[0x10]=0xBE; buf[0x11]=0x0B; buf[0x12]=0x3F; buf[0x13]=0x00; buf[0x14]=0xE9;
    buf[0x15]=0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert!(m.out.text().contains('é'), "é reached the output sink: {:?}", m.out.text());
}

#[test]
fn check_unicode_reports_printable_and_receivable() {
    let mut buf = sample_story(5);
    // check_unicode 0x00E9 -> G0  (EXT:0x0C, [Large], store)
    buf[0x10]=0xBE; buf[0x11]=0x0C; buf[0x12]=0x3F; buf[0x13]=0x00; buf[0x14]=0xE9; buf[0x15]=0x10;
    buf[0x16]=0xBA;
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    assert_eq!(m.global(0), 3, "valid scalar: printable|receivable = 3");
}
```

(If the output sink accessor is not `m.out.text()`, grep the test module for how existing print tests read output — e.g. `print_char` tests — and match that.)

- [ ] **Step 2: Run to verify they fail** — `cargo test -p zvm print_unicode check_unicode` → no-op fall-through, fails.

- [ ] **Step 3: Implement the arms** (in `exec_ext`):

```rust
// EXT:0x0B print_unicode — output an arbitrary Unicode codepoint.
0x0B => {
    let cp = ops.first().copied().unwrap_or(0) as u32;
    let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
    let mut b = [0u8; 4];
    self.print_text(ch.encode_utf8(&mut b));
    StepResult::Continue
}
// EXT:0x0C check_unicode — bit0: can print, bit1: can input. We render and read
// UTF-8, so any valid scalar value is both (3); invalid -> 0.
0x0C => {
    let cp = ops.first().copied().unwrap_or(0) as u32;
    let val = if char::from_u32(cp).is_some() { 3 } else { 0 };
    self.do_store(store, val);
    StepResult::Continue
}
```

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): print_unicode / check_unicode (EXT:0x0B/0x0C)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 4: encode_text (VAR:0x1C)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — new `exec_var` arm `0x1C`.
- Consumes: `text::encode::encode_word(text: &str, version: u8) -> Vec<u8>`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn encode_text_writes_packed_word() {
    let mut buf = sample_story(5);
    // Lay out a ZSCII source word "sword" at 0x40 (dynamic memory), and a 6-byte
    // coded-text buffer at 0x50. encode_text 0x40, 5, 0, 0x50.
    for (i, b) in b"sword".iter().enumerate() { buf[0x40 + i] = *b; }
    // encode_text (VAR:0x1C). 4 operands [text,length,from,coded]: type byte
    // 0b01_01_01_01 = 0x55 (four Small/Large smalls). Use small consts (<256).
    buf[0x10]=0xEC;        // VAR form, opcode 0x1C -> 0xE0|0x1C = 0xFC? see note
    // NOTE: VAR opcodes are encoded 0b111_xxxxx for the VAR-count form; opcode
    // byte = 0xE0 | opcode. 0xE0 | 0x1C = 0xFC. Use 0xFC, then a type byte, then operands.
    buf[0x10]=0xFC; buf[0x11]=0x55; buf[0x12]=0x40; buf[0x13]=5; buf[0x14]=0; buf[0x15]=0x50;
    buf[0x16]=0xBA; // quit
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    let expected = zvm::text::encode::encode_word("sword", 5);
    for (i, b) in expected.iter().enumerate() {
        assert_eq!(m.mem.read_byte(0x50 + i as u32), *b, "coded byte {i}");
    }
}
```

(Confirm the `encode_word` path is reachable from the test module — it is `pub` in `text::encode`. If the VAR opcode byte for an explicit-store-less VAR is different in this codebase, mirror an existing VAR test, e.g. the `storew`/`print_table` tests, for the exact encoding.)

- [ ] **Step 2: Run to verify it fails** — `cargo test -p zvm encode_text_writes_packed_word` → unimplemented VAR fall-through, fails.

- [ ] **Step 3: Implement the arm** (in `exec_var`):

```rust
// VAR:0x1C encode_text zscii-text length from coded-text — encode `length`
// ZSCII bytes at zscii-text+from to the packed dictionary form at coded-text.
0x1C => {
    let src = ops.first().copied().unwrap_or(0) as u32;
    let length = ops.get(1).copied().unwrap_or(0) as u32;
    let from = ops.get(2).copied().unwrap_or(0) as u32;
    let coded = ops.get(3).copied().unwrap_or(0) as u32;
    let mut s = String::new();
    for i in 0..length {
        let b = self.mem.read_byte(src + from + i);
        s.push(crate::text::zscii_to_char(b)); // mirror the read path's ZSCII decode
    }
    let packed = crate::text::encode::encode_word(&s, self.mem.version());
    for (i, b) in packed.iter().enumerate() {
        self.mem.write_byte(coded + i as u32, *b);
    }
    StepResult::Continue
}
```

(If `zscii_to_char` is private/elsewhere, use the same helper `print_char` uses to map ZSCII→char; grep `zscii_to_char`. ASCII letters map 1:1, which the test relies on.)

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): encode_text (VAR:0x1C)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 5: tokenise (VAR:0x1B)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — new `exec_var` arm `0x1B`. Reuses the parse-buffer-writing logic from `supply_line` (~1271) and `dictionary::{load, Dictionary::tokenise}`.

**Interfaces:**
- Consumes: `dictionary::load(&Memory) -> Dictionary`, `Dictionary::tokenise(&self, &Memory, &str) -> Vec<Token>` where `Token { dict_addr: u16, len: u8, text_pos: u8 }`.
- Produces: factor the parse-buffer write into a reusable `fn write_parse_buffer(&mut self, parse: u32, tokens: &[Token], flag: bool)` so `supply_line` and `tokenise` share it (DRY). If `supply_line` already inlines this, extract it in this task and have both call it.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn tokenise_parses_a_dictionary_word_into_parse_buffer() {
    // Use a story whose dictionary contains a known word. sample_story's dict
    // content must be known; if sample_story has no usable dictionary, build a
    // minimal one the same way the existing read/tokenise tests do (mirror them).
    let mut buf = sample_story(5);
    // text buffer at 0x40: v5 form is [max_len][cur_len][chars...]; for tokenise
    // the text buffer holds the already-entered input. Write a known dict word.
    // (Mirror the existing read/supply_line test's text-buffer layout.)
    // parse buffer at 0x60: byte0 = max words (e.g. 4).
    buf[0x60] = 4;
    // tokenise text=0x40 parse=0x60 (dict=0 default, flag=0): VAR:0x1B = 0xFB.
    buf[0x10]=0xFB; buf[0x11]=0x5F; buf[0x12]=0x40; buf[0x13]=0x60; // [Small 0x40, Small 0x60, omit, omit]
    buf[0x14]=0xBA;
    let mem = Memory::new(buf).unwrap();
    let mut m = Machine::new(mem);
    m.state.pc = 0x10;
    run_until_quit(&mut m);
    // parse buffer byte1 = number of words parsed (>=1 for a known word).
    assert!(m.mem.read_byte(0x61) >= 1, "at least one token parsed");
    // first token: dict addr (word at 0x62) nonzero for an in-dictionary word.
    assert_ne!(m.mem.read_word(0x62), 0, "known word resolved to a dict entry");
}
```

**IMPORTANT for the implementer:** before writing the test body, read the existing
`read`/`supply_line` tests (grep the test module for `supply_line` / `tokenise` /
`parse_buf`) and reuse their exact text-buffer + dictionary setup. The assertion
shape above (parse byte1 ≥ 1, first dict addr ≠ 0) is the contract; the buffer
layout must match the codebase's `read` conventions for the story version.

- [ ] **Step 2: Run to verify it fails** — `cargo test -p zvm tokenise_parses` → unimplemented VAR fall-through, fails.

- [ ] **Step 3: Implement the arm**

First, if `supply_line` (~1271) inlines the parse-buffer write, extract it:

```rust
/// Write tokens into a parse buffer in the standard format:
/// [byte0 = max words][byte1 = count]; then per word: dict-addr(word), len(byte), pos(byte).
fn write_parse_buffer(&mut self, parse: u32, tokens: &[crate::dictionary::Token], flag: bool) {
    let max = self.mem.read_byte(parse) as usize;
    let n = tokens.len().min(max);
    self.mem.write_byte(parse + 1, n as u8);
    for (i, t) in tokens.iter().take(n).enumerate() {
        // flag set + word not in dictionary (dict_addr==0): leave the slot untouched.
        if flag && t.dict_addr == 0 { continue; }
        let base = parse + 2 + (i as u32) * 4;
        self.mem.write_word(base, t.dict_addr);
        self.mem.write_byte(base + 2, t.len);
        self.mem.write_byte(base + 3, t.text_pos);
    }
}
```

Update `supply_line` to call `write_parse_buffer` (same behaviour). Then add the arm:

```rust
// VAR:0x1B tokenise text parse [dictionary] [flag] — lex the text buffer into
// the parse buffer, like the lexing half of `read`.
0x1B => {
    let text_buf = ops.first().copied().unwrap_or(0) as u32;
    let parse = ops.get(1).copied().unwrap_or(0) as u32;
    let _dict_addr = ops.get(2).copied().unwrap_or(0); // 0 = standard dictionary
    let flag = ops.get(3).copied().unwrap_or(0) != 0;
    // Read the already-entered text out of the text buffer (mirror supply_line's
    // text extraction for this version — v5 stores cur_len at byte1 then chars).
    let text = self.read_text_buffer(text_buf); // extract/reuse helper from supply_line
    let dict = crate::dictionary::load(&self.mem); // honour _dict_addr if a custom
                                                   // dictionary is supplied (optional; 0 = standard)
    let tokens = dict.tokenise(&self.mem, &text);
    self.write_parse_buffer(parse, &tokens, flag);
    StepResult::Continue
}
```

**Implementer note:** reuse `supply_line`'s existing text-extraction and `dict.tokenise` call verbatim — factor a `read_text_buffer(text_buf) -> String` helper from it if not already present, so `tokenise` and `read` share the same lexing. Honouring a non-zero custom `dictionary` operand is optional (the default-dictionary path covers the common case); if `dictionary::load` cannot target an arbitrary address, leave a `// TODO custom dict addr` and use the standard dictionary (note it in the report).

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): tokenise (VAR:0x1B) reusing the read lexing + parse-buffer write

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 6: Custom alphabet table (header 0x34, v5+)

**Files:**
- Modify: `crates/zvm/src/text/decode.rs` — the A0/A1/A2 glyph lookup (~91) to consult a custom table when header word 0x34 is nonzero (v5+).

**Interfaces:**
- The decoder must read the alphabet glyph from `mem` at `0x34`-pointed table when present. The table is 78 ZSCII bytes: rows A0 (0..26), A1 (26..52), A2 (52..78). A2 position 0 and 1 are special (escape / newline) and are NOT taken from the table — keep the existing handling for those.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn custom_alphabet_table_overrides_default_glyphs() {
    // Build a v5 story with header 0x34 pointing at a custom 78-byte table where
    // A0[0] = 'z' (instead of 'a'). Decoding Z-char 6 (first A0 letter) yields 'z'.
    let mut buf = sample_story(5);
    let tbl: u32 = 0x0200; // somewhere in static memory we control in sample_story
    buf[0x34] = (tbl >> 8) as u8; buf[0x35] = (tbl & 0xFF) as u8;
    // Fill default-ish, but set A0[0] = b'z'.
    let a0 = b"zbcdefghijklmnopqrstuvwxyz";
    let a1 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let a2 = b"\x00\n0123456789.,!?_#'\"/\\-:()";
    for (i, &c) in a0.iter().enumerate() { buf[tbl as usize + i] = c; }
    for (i, &c) in a1.iter().enumerate() { buf[tbl as usize + 26 + i] = c; }
    for (i, &c) in a2.iter().enumerate() { buf[tbl as usize + 52 + i] = c; }
    let mem = Memory::new(buf).unwrap();
    // Decode a string of a single A0[0] Z-char (Z-char value 6) and assert 'z'.
    // Use the crate's decode entry point the read/print path uses (grep decode.rs
    // for the pub fn, e.g. decode_string(&mem, addr) or zstr_at). Encode a packed
    // word containing one Z-char 6 + padding (5,5) at a known addr and decode it.
    // (Mirror an existing decode.rs test for the exact call + packing.)
    // assert_eq!(decoded, "z");
}
```

**Implementer:** complete this test using the existing `decode.rs` test pattern
(there are decode tests already — mirror their packed-word construction and the
exact public decode entry point). The contract: with the custom table installed,
the first A0 letter decodes to `'z'`; without it (default), `'a'`.

- [ ] **Step 2: Run to verify it fails.**

- [ ] **Step 3: Implement** — in `decode.rs`, where `A0/A1/A2[idx]` is indexed (~91), first resolve the table source once per decode:

```rust
// v5+ custom alphabet table (header 0x34). When present, glyph rows come from
// the table (78 ZSCII bytes: A0 0..26, A1 26..52, A2 52..78). A2 specials
// (positions 0 = escape, 1 = newline) keep their built-in handling.
let custom = if mem.version() >= 5 {
    let p = mem.read_word(0x34) as u32;
    (p != 0).then_some(p)
} else { None };
// ...
let ch = match custom {
    Some(tbl) => {
        let row = alphabet as u32; // 0,1,2
        mem.read_byte(tbl + row * 26 + idx as u32)
    }
    None => match alphabet { 0 => A0[idx], 1 => A1[idx], _ => A2[idx] },
};
```

Keep the existing A2[0]/A2[1] special-case handling ahead of this lookup so the
escape/newline behaviour is unchanged.

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/text/decode.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): custom alphabet table (header 0x34, v5+) in text decode

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 7: Terminating-characters table (header 0x2E, v5+)

**Files:**
- Modify: `crates/zvm/src/cpu/exec.rs` — the v5 `read` termination/store path (~1266-1290).

**Interfaces:**
- When header word 0x2E is nonzero (v5+) it points to a zero-terminated list of ZSCII terminating characters (255 = any function key). On termination, store the actual terminating character (not hard-coded 13). Enter (13) always terminates.

- [ ] **Step 1: Write the failing test**

The cleanest unit is a helper that decides whether a character terminates input
and what is stored. Factor it so it is testable without the full input loop:

```rust
#[test]
fn terminating_chars_table_is_honoured() {
    // Build a v5 story with header 0x2E -> a table [0x81, 0x00] (function key 129).
    let mut buf = sample_story(5);
    let tbl: u32 = 0x0200;
    buf[0x2E] = (tbl >> 8) as u8; buf[0x2F] = (tbl & 0xFF) as u8;
    buf[tbl as usize] = 0x81; buf[tbl as usize + 1] = 0x00;
    let mem = Memory::new(buf).unwrap();
    let m = Machine::new(mem);
    // is_terminator(ch) consults the table + always-Enter.
    assert!(m.is_terminator(13), "Enter always terminates");
    assert!(m.is_terminator(0x81), "listed function key terminates");
    assert!(!m.is_terminator(b'a' as u16), "ordinary char does not terminate");
}
```

- [ ] **Step 2: Run to verify it fails** (`is_terminator` missing).

- [ ] **Step 3: Implement** — add the helper and use it in the read path:

```rust
/// v5+: does `ch` terminate line input? Enter (13) always does; otherwise, if a
/// terminating-characters table (header 0x2E) is present, any listed char does
/// (255 in the table = any function key, i.e. ch >= 129).
pub fn is_terminator(&self, ch: u16) -> bool {
    if ch == 13 { return true; }
    if self.mem.version() < 5 { return false; }
    let mut p = self.mem.read_word(0x2E) as u32;
    if p == 0 { return false; }
    loop {
        let t = self.mem.read_byte(p) as u16;
        if t == 0 { return false; }
        if t == 255 { if ch >= 129 { return true; } }
        else if t == ch { return true; }
        p += 1;
    }
}
```

In the read path (~1290), where it currently stores `13` for v5+, store the actual
terminating character that ended the line (Enter → 13; a function-key terminator →
its ZSCII code). When input ends on Enter (the common host path), keep storing 13.
Thread the terminating character through from where the line is supplied; if the
host only ever supplies Enter-terminated lines today, store 13 as before but route
it through `is_terminator`-aware logic so a future function-key terminator stores
its own code. Document this in the report.

- [ ] **Step 4: Run + suite** — `cargo test -p zvm` → PASS, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git -C /Volumes/Videos/Source/lanthorn add crates/zvm/src/cpu/exec.rs
git -C /Volumes/Videos/Source/lanthorn commit -m "feat(zvm): terminating-characters table (header 0x2E, v5+)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

## Notes for the executor

- Order matters only for shared-file coalescing: Tasks 1-5 and 7 all edit
  `crates/zvm/src/cpu/exec.rs`; do them sequentially. Task 6 edits `text/decode.rs`.
- Every test uses the real harness. Where a test's exact instruction encoding or
  buffer layout is uncertain, MIRROR the nearest existing test (grep the test
  module): EXT → `log_shift`; branch → `je_branch_taken_and_not_taken`; VAR with a
  store/buffer → `storew`/`print_table`; read/lexing → the `read`/`supply_line`
  tests; decode → the existing `decode.rs` tests. The assertion contracts above are
  the spec; adapt only the byte-level setup to the codebase's conventions.
- If a reused helper has a different name than assumed (`zscii_to_char`,
  `read_text_buffer`, `m.out.text()`, `Memory::len`), grep for the real one and use
  it — do not invent. Note any such substitution in the task report.
- 0 warnings is a gate. Remove any symbol your change orphans.
