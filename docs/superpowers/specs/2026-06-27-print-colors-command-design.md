# `/print-colors` Command — Print the Current Color Scheme

**Date:** 2026-06-27
**Status:** Approved, ready for planning
**Relation:** companion to the per-game default-reset fix
(`2026-06-27-per-game-default-reset-design.md`); shares the same wave.

## Goal

A slash command that prints the **live** resolved color scheme to the transcript
— every styleable selector with its foreground, background, and attributes,
grouped by category — so a user can see exactly what the active theme resolves to
(the same view shown conversationally as a table).

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

### 1. Formatter — `style::describe_scheme(cs: &ColorScheme) -> Vec<String>`

A new public, testable function in `style.rs` that returns the lines to print:

- For each `(title, selectors)` in `SELECTOR_GROUPS`, emit a header line for the
  group, then one line per selector:
  `  <selector>: fg=<fg> bg=<bg><attrs>`
  where `<fg>`/`<bg>` come from `style_for_selector(cs, selector)` →
  `color_to_str` for `Some(color)`, or the literal `default` when the field is
  `None`; and `<attrs>` lists any set modifiers (`bold`, `italic`, `underline`,
  `dim`, `reversed`) — omitted entirely when none are set.
- The reserved `border` selector (no color field) is skipped.
- Exact header format and column layout are an implementation detail of this
  function; the test pins the essential content (group titles present; a known
  selector line shows the expected fg/bg/attrs), not the spacing.

`describe_scheme` and `color_to_str` live in the same module (`style.rs`), so no
visibility change is needed.

### 2. Command — `print-colors`

Add one `CommandSpec` to `COMMANDS`:

- name: `print-colors`, category: `Category::Style`, context: `Context::Global`,
  usage: `print-colors`, description: e.g. "print the current color scheme",
  dispatch: returns a new `SlashOutcome::PrintColors`.
- Bump the registry-count assertion (47 → 48) and the descriptive comment
  (Style 4 → 5).

### 3. Output wiring

- Add `SlashOutcome::PrintColors` and handle it in `dispatch_slash_outcome`
  (main.rs): compute `app::style::describe_scheme(&state.colors)` and push each
  line into the transcript as a `Meta` entry, mirroring how `SlashOutcome::Help`
  pushes help lines. Uses the live `state.colors`, so the output reflects the
  active theme and any unsaved editor changes already applied to the live scheme.

## Testing

- `describe_scheme`: on `ColorScheme::terminal_default()`, the result contains
  each group title from `SELECTOR_GROUPS`, and the `room` line shows `fg=white`
  and `bg=reset`, the `connector` line shows `fg=cyan`, and a selector with an
  attribute (e.g. `map_layer_tab_active`) shows `bold`. Non-vacuous: asserts
  specific token content, not just non-emptiness.
- Registry: `find_command("print-colors")` is `Some` with `Category::Style`; the
  count assertion is 48.
- Parse: `parse("print-colors", '/')` yields `SlashOutcome::PrintColors`.

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
