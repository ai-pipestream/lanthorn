# zvm-cli Polish — escape keys, [MORE] paging, IFID aux, + two TTY fixes — Design

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Crate:** `crates/zvm-cli` + a small shared `zvm::ifid` move (no other engine change)
**Builds on:** the shipped DOS-parity screen model (`2026-06-27-zvm-cli-screen-model-design.md`).

## Goal

Finish the zvm-cli DOS-parity polish: decode arrow/function keys for `read_char`,
add `[MORE]` paging, key aux storage by IFID, and fix two TTY bugs in the
just-shipped screen model (no screen reset on startup; terminal echo lost after a
raw-input menu like the Leather Goddesses hint).

## 1. Multi-byte escape decoding for `read_char`

`read_char_input` currently reads exactly one raw byte. Extend it (TTY only) to
decode terminal escape sequences into Z-machine input codes (ZMSD §3.8):

- After reading the first byte, if it is `ESC` (0x1B), read the continuation
  with a brief timeout (`stty min 0 time 1` ≈ 0.1s, up to ~7 bytes) so a lone
  `ESC` keypress doesn't block.
- Pure mapper `decode_escape_seq(seq: &[u8]) -> Option<u8>` maps the bytes after
  `ESC`:
  - `[A`→129 (up), `[B`→130 (down), `[C`→132 (right), `[D`→131 (left); the same
    for the `O`-prefixed variants (`OA`..`OD`).
  - `OP`/`OQ`/`OR`/`OS`→133/134/135/136 (F1–F4).
  - Anything else → `None`.
- On `None` (or no continuation bytes), fall back to the raw `ESC` byte (27).
- Piped input is unchanged (reads a byte from a line).

`decode_escape_seq` is unit-tested; the stty/read remains I/O exercised by manual
smoke.

## 2. `[MORE]` paging

When the lower window fills on an interactive terminal, pause with a `[MORE]`
prompt until a key is pressed — the DOS behavior — so long output isn't missed.

- The `StdoutOutput` sink gains paging state: `is_tty`, `paging: bool` (on only
  when **both** stdin and stdout are TTYs — never when piped, or it would
  deadlock the harness), `page_height: u16`, and `lines: u16` (lines emitted
  since the last reset).
- In `print`/`print_styled`, count `'\n'` in the emitted text. When `lines`
  reaches `page_height - 1`, emit a reverse-video `[MORE]` (`\x1b[7m[MORE]\x1b[0m`),
  read one keypress (the shared raw key-reader), erase the prompt
  (`\r\x1b[2K`), and reset `lines = 0`.
- `page_height` = `term_rows - top_region_rows - 1`. `main` sets it on the sink
  (via the existing `as_any_mut` downcast) whenever the pinned region height
  changes and at startup; a sane floor of `2` avoids a zero/looping page.
- `main` resets `lines = 0` after every input prompt (the player has caught up).
- `--no-more` (or `--no-page`) disables paging entirely (consistent with
  `--no-status`/`--no-aux`); paging is also implicitly off when not a TTY.
- The key-reader used here is the same shared raw-read helper as `read_char`
  (§5), so it honors the capture-once terminal-restore model.

Testing: a pure helper `should_page(lines, page_height) -> bool` and the
`[MORE]`/erase strings are unit-tested; the blocking read is manual.

## 3. IFID-keyed aux storage

Today aux lives at `<story-stem>.aux`, so renaming the story orphans it. Key it
by IFID instead.

- **Move** `compute_ifid(story: &[u8]) -> String` from `crates/app/src/ifid.rs`
  into a new `crates/zvm/src/ifid.rs` (`pub mod ifid;` in the zvm lib). Re-export
  it from the app's `ifid` module (`pub use zvm::ifid::compute_ifid;`) so the
  app's call sites and its existing tests are unchanged. The app-specific
  `map_path`/`archive_path` stay in the app.
- `aux::aux_path` becomes `aux_path(story_path: &Path, ifid: &str) -> PathBuf` =
  `<story-dir>/<sanitized-ifid>.aux`, where sanitize replaces any char that is
  not ASCII-alphanumeric / `-` / `_` with `_` (mirroring `app::aux_store`).
- `main` computes the IFID once from the story bytes and threads it into
  `aux_preload`/`aux_flush`.
- No migration of existing `<stem>.aux` files (the feature shipped hours ago;
  effectively none exist). The aux file stays next to the story (DOS-like).

Testing: `aux_path` maps `(dir/story.z5, "ZCODE-1-840726-ABCD")` →
`dir/ZCODE-1-840726-ABCD.aux`, and sanitizes unsafe characters; `compute_ifid`
keeps its existing tests (now in zvm).

## 4. Bug fix — reset the screen on startup (TTY)

The screen model never clears the terminal, so existing scrollback is overwritten
and intermixed with the pinned region. Add `ScreenView::start() -> String` that
returns `\x1b[2J\x1b[H` (clear + home) when `is_tty && !no_status`, else empty.
`main` writes it once before the run loop. Never emitted when piped or under
`--no-status` (keeps the harness/legacy output byte-clean).

## 5. Bug fix — own terminal state; restore reliably (echo after raw input)

After a raw-input menu (the hint screen's `read_char` navigation) the terminal
is left with `-echo`, so the next line prompt isn't echoed. Root cause: each
`read_char_input` snapshots the mode with `stty -g` and restores to that
snapshot, which can capture a non-cooked mode.

Fix — capture once, restore to the known-good mode:

- At startup, if stdin is a TTY, capture the original mode once:
  `orig = stty -g`.
- The raw key-reader (shared by `read_char` §1 and `[MORE]` §2) sets raw-no-echo,
  reads (decoding escapes), then restores to **`orig`** — not a per-call
  snapshot.
- On exit (the `Quit` arm and any error/`process::exit` path), restore `orig`
  and reset the scroll region (extend the existing `view.leave()` emission with
  a terminal-mode restore). The interpreter always returns the terminal to the
  original cooked+echo mode.
- `orig` is threaded through `main` (a small owned value/handle); when stdin is
  not a TTY it is `None` and all of this is skipped.

This also makes the per-call `stty -g` snapshot unnecessary (removed).

## Out of scope

- Decoding the full function-key/keypad range (only arrows + F1–F4); other
  sequences fall back to raw `ESC`.
- Windows terminal handling (`stty`/ANSI assume a Unix TTY, as the shipped model
  already does).
- A signal handler to restore the terminal on SIGINT/SIGTERM (raw mode has
  `ISIG` cleared during the brief read window; normal exit paths restore).

## Global constraints

- 0 warnings (`cargo build`, `cargo doc --no-deps`) + full `cargo test --workspace`
  green per task.
- `zvm-cli` stays zero-dependency (std only). The only engine change is moving
  `compute_ifid` into `zvm` (pure, no new behavior); the app re-exports it so its
  tests stay green.
- TTY-only effects (`[MORE]`, escape decoding, screen reset, raw mode) MUST be
  inert when piped — the headless harness stays deterministic and never blocks.
  `--no-status` output stays byte-identical to legacy; `--no-more`/`--no-aux`
  opt-outs behave as named.
- Commit-only on local `main`; one commit per task (TDD). No push.
- Commit trailers, every commit (no backticks in the body):
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`.
- Do not edit `TODO.md` during the wave.
