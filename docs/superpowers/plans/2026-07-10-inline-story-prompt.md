# Inline story `>` prompt (default), dedicated command bar as an option

**Goal:** Make lanthorn input happen at the story's own inline `>` prompt in the transcript
(classic terminal-interpreter style) BY DEFAULT, and make today's dedicated bottom command
bar an opt-in config flag.

**User decisions (locked):**
- Inline story prompt is the DEFAULT; the command bar is the opt-in.
- Config flag lives in the config file (and the settings screen).
- Autocomplete suggestions in inline mode render on a **dynamic line just below the inline
  prompt** (one row reserved only when suggestions exist), mirroring today's suggestion line.

**Crate:** only `crates/app`. Commit trailers on every commit:
```
Quest: SQ-0264
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
```
Branch off `main`: `sq-0264-inline-prompt`. Stage only edited source files by path (tree has
pre-existing untracked files — never `git add -A`).

---

## Architecture facts (established, do not re-derive)

- Input buffer: `AppState.input: String` (`state.rs:1034`, init `state.rs:1378`). Edits:
  `push_input_char` (`state.rs:1990`), `backspace` (`state.rs:1995`), `take_input` (`state.rs:2000`).
- Dedicated bar render: `render/transcript.rs`. `render_transcript(...)` (`transcript.rs:803`)
  reserves bottom `input_rows` (1, or 3 boxed) at `transcript.rs:858-868`; `render_input_content`
  (`transcript.rs:964`) draws `"> "` (`format_input_line`, `transcript.rs:702`) + `state.input`
  + drawn cursor (`transcript.rs:998-1005`, symbol `"_"`, `CURSOR_STYLE` = REVERSED at
  `transcript.rs:52`). Bar hidden in char mode (`transcript.rs:975`).
- `render_middle(...)` (`transcript.rs:1013`) reserves one more bottom row for the
  suggestion/search line (`suggestion_y = middle_bottom - 1`, `transcript.rs:1030`;
  transcript body bottom is `suggestion_y` when a suggestion/search line shows, else
  `middle_bottom`, `transcript.rs:1081-1088`; boxed popup path `transcript.rs:1038-1052`).
  The wrapped visible window comes from `visible_wrapped_lines_kinded(...)` and is drawn
  top-to-bottom at `row_y = transcript_top + i` (`transcript.rs:1185+`); it now also returns
  `first_abs_row` (SQ-0197).
- Prompt stripping: `strip_read_prompt(s)` (`session.rs:556`) removes a trailing `>` that is
  alone or preceded by `\n`. Call sites: Z-machine `session.rs:263`, `:408`, `:1005`; Glulx
  `glulx_session.rs:300`, `:537`, `:554`. This is what hides the game's `>` today.
- Submit + echo: `SubmitCommand` handled at `main.rs:3493` → `take_input` (`main.rs:3513`),
  slash intercept (`main.rs:3526`), `session.submit` (`main.rs:3550`), `finish_command_turn`
  (`main.rs:3551`). Echo is `state.push_transcript_kind(&format!("> {}", cmd), Input)` at
  `main.rs:4575`.
- Char mode: `state.char_mode` set each frame (`main.rs:2270`); keys intercepted at
  `main.rs:3083-3109`; bar suppressed. Inline mode does not change char handling.
- Config bool pattern (mirror `mouse`): default (`config.rs:491`), field (`config.rs:361`),
  from_file merge (`config.rs:566`), TOML serialize (`config.rs:629`); settings screen row
  (`config_screen.rs`, `CONFIG_ROWS` at `:12`, value arm ~`:178`, `bool_str` `:183`) + the two
  toggle fns in `input.rs`: `config_toggle_or_edit` (`input.rs:3857`) and `config_cycle`
  (`input.rs:3905`). Row indices are HARDCODED — append as the last row (index 14).

---

## Task 1 — Config flag `command_bar` (default false)

Mirror `mouse` exactly, appended as the LAST config row so hardcoded indices don't shift.

- `config.rs`: add field `#[serde(default)] pub command_bar: bool,` near `mouse` (`:361`);
  `Default` impl `command_bar: false` (`:491` area); `from_file` merge
  `cfg.command_bar = from_file.command_bar;` (`:566` area); TOML serialize
  `doc["command_bar"] = toml_edit::value(cfg.command_bar);` (`:629` area).
- `config_screen.rs`: append `("command_bar", ConfigRowKind::Bool)` as the last `CONFIG_ROWS`
  entry (index 14); add a value-render arm `14 => bool_str(cfg.command_bar)` mirroring mouse.
- `input.rs`: in `config_toggle_or_edit` add `14 => working.command_bar = !working.command_bar,`
  and in `config_cycle` add the index-14 bool arm (mirror mouse's arms exactly).

**Tests:** `config.rs` — `command_bar` defaults false; round-trips through TOML.
**Commit:** `feat(app): add command_bar config option (default off = inline prompt) (SQ-0264)`.

---

## Task 2 — Conditional prompt stripping (keep the game's `>` in inline mode)

The session must keep the game's trailing `>` when inline (command_bar=false).

- Add `strip_prompt: bool` field to `GameSession` (`session.rs`) and `GlulxSession`
  (`glulx_session.rs`), defaulting `true` (preserve current behavior for all existing
  constructors/tests — set it `true` in each `Self { ... }` literal in `new`).
- Add an `Engine` trait method (`engine.rs`, near other setters like `set_screen_size`):
  ```rust
  /// When false, the game's own trailing `>` read prompt is preserved in the
  /// transcript (inline-prompt mode) instead of being stripped for the app's
  /// dedicated input bar. Default true.
  fn set_strip_prompt(&mut self, _on: bool) {}
  ```
  Override in both sessions to set the field.
- Gate the strip: at each `strip_read_prompt(&raw)` transcript call site
  (`session.rs:263`, `:408`; `glulx_session.rs:300`, `:537`), use
  `if self.strip_prompt { strip_read_prompt(&raw) } else { &raw }`. The clamp-length call
  sites (`session.rs:1005`, `glulx_session.rs:554`) must use the SAME kept length — compute
  `let kept = if self.strip_prompt { strip_read_prompt(&raw).chars().count() } else { raw.chars().count() }`
  and reuse it, so `clamp_runs`/`trim_elems_to_len` stay aligned with the emitted text.
- `main.rs`: right after creating the engine/session, call
  `engine.set_strip_prompt(state.config.command_bar);` (strip only in command-bar mode).

**Tests:** `session.rs`/`glulx_session.rs` — with `strip_prompt=false`, a banner ending in
`"...\n>"` keeps its trailing `>` in `take_transcript`; with `true` (default) it is stripped
(existing behavior). Add one test each side.
**Commit:** `feat(app): preserve the game's inline read prompt when command_bar is off (SQ-0264)`.

---

## Task 3 — Submit/echo appends to the prompt line in inline mode

In inline mode the typed command must join the game's `>` line (so `>look` persists), instead
of pushing a separate `"> cmd"` echo line.

- Add `state.rs`: `pub fn append_to_last_transcript_line(&mut self, text: &str)` — appends
  `text` to the last `transcript` line in place; if `transcript` is empty, pushes `text` as a
  new line. Keeps the parallel arrays consistent (it edits only the last `String`; the last
  line's `transcript_runs`/`kinds`/`images`/`styles` are unchanged — the appended chars have
  no runs, which renders in the input style; acceptable). Return nothing.
- `main.rs` `finish_command_turn` (`:4575`): replace the unconditional echo with:
  ```rust
  if state.config.command_bar {
      state.push_transcript_kind(&format!("> {}", cmd), TranscriptKind::Input);
  } else {
      // Inline mode: the game's own `>` is already the last transcript line; append the
      // typed command to it so `>look` persists in scrollback, then the response follows.
      state.append_to_last_transcript_line(&cmd);
  }
  ```
  (Everything else in `finish_command_turn` — pushing the response transcript — is unchanged.)

**Tests:** `state.rs` — `append_to_last_transcript_line` appends to the last line and is a
no-op-safe push when empty. (The end-to-end echo is covered by the manual smoke.)
**Commit:** `feat(app): append the typed command to the inline prompt line (SQ-0264)`.

---

## Task 4 — Render: drop the bar + draw the inline prompt & cursor

This is the crux. In inline mode: no dedicated bar row; the live `state.input` + cursor draw
right after the last transcript row's text (the game's `>`); suggestions get a dynamic bottom
row below it.

- `render_transcript` (`transcript.rs:803`): when `!state.config.command_bar`, set
  `input_rows = 0` (skip the `input_boxed`/`render_input_content` reservation and call
  entirely). The whole bottom region flows into `middle_area`.
- `render_middle` (`transcript.rs:1013`): after the wrapped-row draw loop (`:1185+`), when
  `!state.config.command_bar` and NOT `state.char_mode` and `state.focus == Focus::Game` and
  no overlay open:
  - Identify the last drawn transcript row (the visible row with the greatest `i` whose
    `abs = first_abs_row + i` equals `total_rows - 1`, i.e. the true last line — only draw the
    live input when the last line is actually visible, i.e. the view is at the bottom
    `effective_scroll == 0`; otherwise skip, the user is reading history).
  - Compute that row's screen `row_y` and the column just past its text:
    `col = text_x + wr.text.chars().count() as u16` (clamp to `body_area.right()-1`).
  - Draw a leading space then `state.input` then the cursor `"_"` (CURSOR_STYLE) starting at
    `col`, clipped to `body_area`. Use `state.colors.input_text` for the typed text (and the
    game's colour when honoured — reuse the same resolution `render_input_content` uses).
  - Register the geometry so nothing else clobbers it; this draws over the reserved
    suggestion row only if needed — keep the prompt on the transcript's last row, suggestions
    on the row below (see next).
  - Suggestion line: the existing `suggestion_y` reservation in `render_middle` already sits
    at `middle_bottom - 1`. In inline mode that row is BELOW the transcript body; keep using it
    for suggestions/search exactly as today (the code already draws there when
    `has_suggestions`/`has_search`). The only change is the input prompt is inline (last
    transcript row) rather than in `render_transcript`'s bar.
- Cursor blink/visibility parity: reuse the same guard as the bar cursor
  (`focus == Game && !any_overlay_open()`), plus `effective_scroll == 0`.

Edge cases to handle explicitly:
- If the last line does not end in `>` (game printed no prompt), still draw the live input at
  the end of the last line (append-at-end); acceptable and rare.
- If `state.input` would overflow the row width, clip at `body_area.right()` (no wrap for the
  live line in v1; note as a follow-up).

**Tests (`transcript.rs`):**
- Inline mode: with `command_bar=false`, a transcript whose last line is `">"` and
  `state.input = "look"` renders `>look` on the last row with the cursor cell after it; and NO
  dedicated bottom bar row is drawn (assert the old bar position holds transcript/empty, not
  `"> "`).
- Command-bar mode: with `command_bar=true`, behavior is unchanged — the bottom bar renders
  `"> "` + input (existing tests still pass).
- Scrolled-up: with `effective_transcript_scroll > 0`, the live input is NOT drawn over
  history.

**Commit:** `feat(app): draw the inline story prompt + cursor, drop the bar when off (SQ-0264)`.

---

## Task 5 — Wire-up sweep + verification

- Grep for any other consumer that assumes the dedicated bar exists (e.g. mouse hit-testing of
  the input row, `PaneRects`), and gate on `command_bar`. Check `main.rs` for input-row mouse
  handling and the `char_mode` prompt-hide interplay.
- Confirm char-mode and timed-input paths are unaffected (they suppress the bar today and
  should suppress the inline live-input the same way — the Task 4 guard already excludes
  `char_mode`).

**Verification:**
```bash
cargo test -p app
cargo build --workspace
```
**Manual smoke (add to to-verify):**
- Default run (no `command_bar` in config): the story's `>` shows inline at the end of the
  transcript; typing appears after it; Enter commits `>command` into scrollback and the
  response follows; no bottom bar. Autocomplete suggestions appear on the line below the
  prompt. Scroll up (wheel) → the live input is not drawn over history; scroll back to bottom →
  it returns.
- `command_bar = true` in config: today's dedicated bottom bar returns, and the game's `>` is
  stripped (no double prompt). Toggle it on the settings screen too.
- Char-input game (e.g. a "press any key") and a timed-input game behave as before in both
  modes.

## Notes / deferred
- No live-input line wrapping in v1 (clip). A very long typed command clips at the pane edge.
- Runtime toggle via the settings screen changes future turns; already-stripped scrollback
  isn't retroactively restored (acceptable).
- The appended command chars carry no style runs (render in the input style) — fine for v1.
