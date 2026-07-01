# Z-Machine Capability Audit

**Date:** 2026-07-01
**Scope:** The Z-machine interpreter (`crates/zvm/`) and how the two hosts
(`crates/app/`, `crates/zvm-cli/`) render it. The Glulx/Glk gestalt equivalent
is a **separate** follow-up (see §7) — this audit is Z-machine only.
**Method:** Every claim below is grounded in the current source at a cited
`file:line`. "Advertise" = the interpreter-writable header capability bits set
in `zvm::screen::init_header_caps` (`crates/zvm/src/screen.rs:306`).

## 1. Headline conclusion

**The header advertising is already accurate and appropriately conservative.**
There is no bit we secretly support but hide, and (with two exceptions below)
nothing we advertise that we cannot deliver. The remaining value of this audit
is therefore:

1. A **feature-gap inventory** (§5) with an add/defer/skip recommendation each.
2. **Two genuine correctness bugs** surfaced while auditing (§4) — these are the
   only items that warrant near-term code changes.
3. Minor **request-bit hygiene** nits in Flags2 (§3.3).

## 2. Version support

Loader accepts **v3, v4, v5, v7, v8**. Rejected: v1/v2 (`ZError::UnsupportedVersion`),
v6 (`ZError::GraphicalV6`) — `crates/zvm/src/header.rs:34-47`. Every v6-only
capability below is therefore moot by construction (the story never loads).

## 3. Header capability-bit audit

### 3.1 Flags 1 (byte 0x01) — `screen.rs:316-338`, `advertise_colour` `:382`

| Ver | Bit | Meaning | We set | Verdict |
|-----|-----|---------|--------|---------|
| v3 | 1 | time vs score status | left to game | ✅ correct |
| v3 | 4 | status line NOT available | cleared (we have one) | ✅ |
| v3 | 5 | screen-splitting available | **set** | ✅ (split_window works) |
| v3 | 6 | variable-pitch default | cleared (fixed) | ✅ |
| v4+ | 0 | colour available | set iff `honor_game_colours` | ✅ gated, matches impl |
| v4+ | 1 | picture display | cleared | ✅ (v6 rejected; no graphics) |
| v4+ | 2 | boldface available | **set** | ✅ (`set_text_style` FULL) |
| v4+ | 3 | italic available | **set** | ✅ |
| v4+ | 4 | fixed-space font | **set** | ✅ |
| v4+ | 5 | sound effects available | cleared | ✅ see note ‡ |
| v4+ | 7 | timed keyboard input | cleared | ✅ (operands ignored — §5) |

‡ **Sound note.** We *do* render the two standard bleeps (`sound_effect` 1/2)
as a visual pulse (`exec.rs:1114-1128`). Bit 5 is specifically *sampled/effect*
sound (the v6-era capability); bleeps are always available regardless of the
bit, so leaving it clear is correct and not a missed advertisement.

### 3.2 Flags 2 (word 0x10) — `screen.rs:347-354`

| Bit | Meaning (game "wants" → int clears if unsupported) | We do | Verdict |
|-----|-----|-------|---------|
| 0 | transcript on | untouched (game/int toggles) | ✅ |
| 1 | force fixed-pitch | untouched | ✅ (we always render on a fixed grid) |
| 2 | (v6) request redraw | untouched | ✅ moot |
| 3 | wants pictures | **cleared** | ✅ |
| 4 | wants/has UNDO | **set for v5+**, cleared <v5 | ⚠️ see §3.3 |
| 5 | wants mouse | **cleared** | ✅ (no `read_mouse`) |
| 6 | wants colours | untouched | ⚠️ see §3.3 |
| 7 | wants sound | **cleared** | ✅ |
| 8 | (v6) wants menus | untouched | ✅ moot (v6 rejected) |

### 3.3 Request-bit hygiene (minor)

- **Bit 4 (undo).** We *force-set* it for v5+ (`screen.rs:350`). It's defined as
  a game-request bit the interpreter *clears* when unavailable; the more correct
  behaviour is to leave the game's request alone and only clear it for <v5.
  Force-setting is harmless in practice (undo *is* available v5+, and `save_undo`
  is v5+-only anyway), and matches how several interpreters treat it as an
  "undo available" advert. **Recommendation: leave as-is; documented.**
- **Bit 6 (colours).** Never cleared even when `honor_game_colours` is OFF. The
  render path gates colour regardless, so a game that proceeds to `set_colour`
  simply has no visible effect — no corruption. **Recommendation: optionally
  clear bit 6 in `advertise_colour` when colour is off, for strict correctness;
  low priority.**

## 4. Correctness bugs surfaced (the actionable findings)

These are *not* header-advertising problems — they are latent behavioural bugs
found while tracing capabilities. Both are small.

### 4.1 `check_unicode` over-reports input capability

`check_unicode` (EXT:0x0C, `exec.rs:1265-1269`) returns `3` (bit0 can-print +
bit1 can-input) for any valid scalar. But the input path is byte-limited:
`supply_char(ch: u8)` (`exec.rs:1690`) and `session.rs:209` cannot carry a
codepoint > 255, and `supply_line` writes raw UTF-8 bytes without ZSCII mapping
(`exec.rs:1649-1667`). So we advertise Unicode *input* we cannot actually
deliver. **Fix (small): report `1` (print-only) until real Unicode input lands**
— honest, and print (which we *do* support via `print_unicode`) stays enabled.

### 4.2 Terminating-characters table parsed but unwired

`is_terminator` (`exec.rs:1441-1466`) correctly reads header 0x2E and handles
`255 = any function key`, but it is **never called on the runtime input path** —
the host hardcodes the terminator to Enter: `session.rs:202` calls
`supply_line(command, 13)`. So a v5+ game that installs a terminating-characters
table (e.g. to catch cursor keys during line input) never sees those keys end
the line. **Fix (moderate): thread the actual terminator from the host's key
event through `supply_line`, consulting `is_terminator`.** Overlaps the existing
TODO "Timed / interrupt input" and the input-key-handling work.

## 5. Feature-gap inventory (add / defer / skip)

Consolidated; cross-referenced to existing TODO items so we don't duplicate.

| Capability | Status | Evidence | Recommendation |
|-----------|--------|----------|----------------|
| Timed input (`read`/`read_char` time+routine) | ABSENT (operands ignored) | `exec.rs:839-851` | **DEFER** — already a standalone TODO ("Timed / interrupt input"); needs a wall-clock tick in the run loop |
| Terminating-chars table wiring | PARTIAL (logic dead) | §4.2 | **ADD (small-moderate)** — see §4.2 |
| Unicode *input* | ABSENT | §4.1 | **DEFER** (feature) + **ADD (tiny)** the honest `check_unicode` report now (§4.1) |
| Mouse input (`read_mouse` EXT:0x16, header ext mouse words) | ABSENT | no arm; `exec.rs:1293` default; `screen.rs:345` | **SKIP** — niche; no target game in library needs it |
| Input streams (`input_stream` VAR:0x14, read from file/table) | ABSENT (diagnostic-only) | `exec.rs:1130-1138`; test `:4418` | **SKIP/DEFER** — replay-from-file; low value for interactive TUI |
| Sampled sound (`sound_effect` n≥3, Blorb audio) | ABSENT (bleeps 1/2 shown visually) | `exec.rs:1110-1128` | **DEFER** — needs Blorb sound loading + an audio backend; bleeps already handled |
| Pictures / v6 graphics (`draw_picture` + picture/window EXT ops) | ABSENT (v6 rejected; `draw_picture` no-op) | `header.rs:47`; `exec.rs:1254`; EXT 0x06-0x1B absent | **SKIP** — v6 story files don't load at all |
| Menus (`make_menu`) | ABSENT | no EXT 0x1A/0x1B arm | **SKIP** — v6-only, moot |
| Output stream 2 (transcript-to-file) | PARTIAL (flag toggled, no file sink) | `exec.rs:965`; flag only | **DEFER** — the app's own always-on transcript UI covers the user need; the *game-driven* stream-2 file is separate |
| Output stream 4 (command log to file) | PARTIAL (flag toggled, no sink) | `exec.rs:974` | **DEFER** — low value |
| Default true fg/bg colours (header-ext words 5/6) | ABSENT (not read) | `memory.rs:28-64` reads only word 3 | **DEFER** — minor; games rarely rely on it given `set_true_colour` works |
| EXT unknown-opcode diagnostics | MISSING (silent `_ => Continue`) | `exec.rs:1293` | **ADD (tiny)** — mirror the VAR one-time diagnostic (`exec.rs:1130`) for observability parity |

### Already-FULL (no gap — listed for completeness)
`set_text_style` (roman/reverse/bold/italic/fixed, cumulative — `exec.rs:937`),
`set_font` 1/3/4 with Font-3 CP437 char-graphics (`exec.rs:1240`, `text/cp437.rs`),
`set_colour`/`set_true_colour` (`exec.rs:1283`), `buffer_mode` (`:947`),
`split_window`/`set_window`/`erase_window`/`set_cursor` screen model,
`save`/`restore` (v3 branch + v5 store + aux-table forms — `exec.rs:1150`),
`save_undo`/`restore_undo` (`:1272`), `print_unicode` (`:1256`), custom Unicode
translation table (`memory.rs:28`), output streams 1/3 (screen + memory table),
`read`/`read_char` basic interactive input.

## 6. Recommended near-term actions

Ordered by value/effort. Only the first two are "bugs"; the rest are the
user's add/defer/skip calls to make.

1. **§4.1** `check_unicode` → report print-only (`1`) until Unicode input exists. *(tiny, correctness)*
2. **§5** EXT unknown-opcode one-time diagnostic. *(tiny, observability)*
3. **§4.2** Wire the terminating-characters table into the host input path. *(small-moderate)*
4. Everything else in §5 is **DEFER/SKIP** per the table — no action unless prioritised.

Header advertising itself needs **no change** (optionally §3.3 bit-6 clear).

## 7. Out of scope — Glulx gestalt equivalent (separate follow-up)

The TODO pairs this Z-machine audit with a Glulx/Glk gestalt audit (`gvm`
`@gestalt` + `glk_gestalt` answering only ~5 selectors). That is a distinct
runtime-capability surface and is **not** covered here; it should get its own
audit pass. Noted so it isn't lost.
