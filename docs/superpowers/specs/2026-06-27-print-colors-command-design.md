# `/print-colors` Command — Print the Current Color Scheme

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Relation:** companion to the per-game default-reset fix
(`2026-06-27-per-game-default-reset-design.md`); shares the same wave.

## Goal

A slash command that prints the **live** resolved color scheme to the transcript
— every styleable selector with its foreground, background, and attributes,
grouped by category — so a user can see exactly what the active theme resolves to
(the same view shown conversationally as a table). An optional `color` argument
renders each selector's line in its **actual** style (a live preview) instead of
the plain `transcript:meta` color.

## Background

- All commands live in one registry: `static COMMANDS: &[CommandSpec]` in
  `crates/app/src/slash.rs` (verb-noun names, a `Category`, a `Context`, and a
  `dispatch: fn(&[&str]) -> SlashOutcome`). The count is asserted by a test
  (currently 47 after Phase 2.2 removed `create-game-style`).
- `SELECTOR_GROUPS` (`style.rs:195`) is the public grouped selector list:
  `&[(category_title, &[selector_name])]` for Map / Transcript / Chrome /
  Dialogs / Upper window / Sound.
- `style_for_selector(cs: &ColorScheme, selector: &str) -> Style` (`style.rs:227`)
  is the public read-accessor returning the resolved `Style` for a selector.
- `color_to_str(Color) -> String` (`style.rs`) renders a color as its token
  (named color, `#rrggbb`, index, or `reset`). It is private to `style.rs` —
  `describe_scheme` lives in the same module, so it can call it directly with no
  visibility change.
- `/help` is the model for transcript output: its dispatch returns
  `SlashOutcome::Help`, and the shared `dispatch_slash_outcome` (main.rs) pushes
  the help text into the transcript as `Meta` lines.

## Design

### 1. Formatter — `style::describe_scheme(cs: &ColorScheme) -> Vec<(String, Option<Style>)>`

A new public, testable function in `style.rs` that returns the lines to print,
each paired with the selector's resolved `Style` (or `None` for a group header):

- For each `(title, selectors)` in `SELECTOR_GROUPS`, emit a header line
  (`(header_text, None)`), then one line per selector
  `(format!("  {selector}: fg={fg} bg={bg}{attrs}"), Some(style_for_selector(cs, selector)))`
  where `<fg>`/`<bg>` come from the selector style → `color_to_str` for
  `Some(color)`, or the literal `default` when the field is `None`; and
  `<attrs>` lists any set modifiers (`bold`, `italic`, `underline`, `dim`,
  `reversed`) — omitted entirely when none are set.
- The reserved `border` selector (no color field) is skipped.
- Exact header format and column layout are an implementation detail of this
  function; the test pins the essential content (group titles present; a known
  selector line shows the expected fg/bg/attrs and carries the expected `Style`),
  not the spacing.

`describe_scheme` and `color_to_str` live in the same module (`style.rs`), so no
visibility change is needed.

### 2. Per-line style override on the transcript (enables `color` mode)

The renderer already styles each transcript line with a single resolved `Style`
(per-line, from the line's `TranscriptKind`). Add an optional per-line override so
a line can be drawn in an arbitrary `Style` instead of its category style:

- `AppState` gains `transcript_styles: Vec<Option<Style>>` (in-memory only; NOT
  persisted to `transcript.json`). Default empty.
- The central `push_transcript_kind` (state.rs:1107) becomes **self-healing**: at
  entry it does `self.transcript_styles.resize(self.transcript.len(), None)` so
  the override vector tracks the transcript length even when other code paths
  reassign `transcript`/`transcript_kinds` wholesale (restore, reset, replay).
  Then, per line pushed, it also pushes `None`.
- New `push_transcript_styled(&mut self, text: &str, kind: TranscriptKind, style: Style)`:
  same self-heal, then per line pushes `transcript`/`transcript_kinds` and
  `Some(style)`.
- Renderer (`render/transcript.rs`, the `filtered_styles` computation ~line 828):
  for each visible line at original index `orig_i`, look up
  `state.transcript_styles.get(orig_i).copied().flatten()` and use it when
  `Some`, otherwise fall back to the existing per-kind style. The lookup is
  defensive (`.get`), so a shorter override vector degrades gracefully to the
  category style — no panic, no required edits at the wholesale-reassignment
  sites.

This mechanism is general (any future command can style specific lines) and adds
no persistence surface.

### 3. Command — `print-colors [color]`

Add one `CommandSpec` to `COMMANDS`:

- name: `print-colors`, category: `Category::Style`, context: `Context::Global`,
  usage: `print-colors [color]`, description: e.g. "print the current color
  scheme (color = show actual colors)",
  dispatch: `|a| SlashOutcome::PrintColors { actual: a.first() == Some(&"color") }`.
- Bump the registry-count assertion (47 → 48) and the descriptive comment
  (Style 4 → 5).

### 4. Output wiring

- Add `SlashOutcome::PrintColors { actual: bool }` and handle it in
  `dispatch_slash_outcome` (main.rs): compute
  `app::style::describe_scheme(&state.colors)`; for each `(line, style_opt)`:
  - **plain mode** (`actual == false`): push the line as a `Meta` entry
    (`push_transcript_kind(line, TranscriptKind::Meta)`), mirroring how
    `SlashOutcome::Help` pushes help lines.
  - **color mode** (`actual == true`): for a selector line (`style_opt` is
    `Some(style)`), push via `push_transcript_styled(line, TranscriptKind::Meta,
    style)`; for a group header (`None`), push as plain `Meta`.
- Uses the live `state.colors`, so the output reflects the active theme and any
  unsaved editor changes already applied to the live scheme.

## Testing

- `describe_scheme`: on `ColorScheme::terminal_default()`, the result's text
  lines contain each group title from `SELECTOR_GROUPS`; the `room` line shows
  `fg=white` and `bg=reset`; the `connector` line shows `fg=cyan`; a selector
  with an attribute (e.g. `map_layer_tab_active`) shows `bold`. Each selector
  line carries `Some(style)` equal to `style_for_selector(cs, selector)`, and
  group headers carry `None`. Non-vacuous: asserts specific token content and the
  paired style, not just non-emptiness.
- Registry: `find_command("print-colors")` is `Some` with `Category::Style`; the
  count assertion is 48.
- Parse: `parse("print-colors", '/')` yields `PrintColors { actual: false }`;
  `parse("print-colors color", '/')` yields `PrintColors { actual: true }`.
- Transcript override: `push_transcript_styled("x", Meta, some_style)` appends a
  line whose `transcript_styles` entry is `Some(some_style)`, and the vectors stay
  equal length; a following `push_transcript_kind` keeps them aligned and pushes
  `None`. After a wholesale `transcript`/`transcript_kinds` reassignment that
  leaves `transcript_styles` short, the next push self-heals (lengths equal
  again).
- Render (TestBackend buffer scrape): in `color` mode a selector line is drawn in
  its actual style — e.g. the `connector` line's cells carry `fg = Cyan` (not the
  `transcript:meta` color); in plain mode the same line carries the
  `transcript:meta` style.

## Out of scope

- A dedicated dialog/overlay for the scheme — transcript Meta lines suffice and
  are scrollable/copyable.
- Printing symbols/glyphs or border styles — colors and attributes only.
- A key binding — it is reachable by name and via the hotkey dialog like any
  command; no default key needed.

## Global constraints

- 0 warnings + full `cargo test -p app` green per task.
- Commit-only on local `main`; TDD wave. No push without explicit instruction.
- Commit trailers, every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV`;
  no backticks in commit bodies.
- Surgical changes; do not edit `TODO.md` during the wave.
- New command registered in the single `COMMANDS` registry (no second
  registration site).
