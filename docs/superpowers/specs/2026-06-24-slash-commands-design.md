# Slash Commands — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** "Add '/' commands for users that prefer that syntax, eg. /panh -1, /panv 1, /save, /load, etc. Tab/autocomplete with suggestions where possible." (#40)
**Related:** unblocks #49 (`/reset [map]`); transcript Story|Meta tagging here is forward-compat with the transcript filter (folded into the console search/export item).
**Touches:** new `crates/app/src/slash.rs`; `main.rs` (submit interception), `state.rs` (status message + transcript-entry category), `complete.rs`/`input.rs` (`/`-mode autocomplete), `render/transcript.rs` (status-line render). No `mapper`/`zvm` changes.

## Goal

When the input line starts with `/`, treat it as an **app command** (not game input): `/save`, `/load`, `/reset`, `/panh -1`, `/zoom in`, etc., with Tab autocomplete over command names. Quiet feedback (status line, not the story transcript), `/help` to list commands.

## Routing

The game-input submit path is `main.rs` ~line 766: `let cmd = state.take_input();` then `session.submit(&cmd)`. Insert: if `cmd.starts_with(prefix)`, route to `slash::run(cmd, …)` and DO NOT call `session.submit`; otherwise unchanged. A slash command consumes the turn's input without advancing `state.turns` or pushing a `> cmd` transcript line.

## Configurable prefix

The trigger character is a **functional config setting** `command_prefix` (default `'/'`, a single char) in `config.toml` — an escape hatch in case a game genuinely takes `/`-leading input. Everything that references the prefix (the submit interception, the `/`-mode autocomplete detection, the displayed `/help`/`/name` hints) reads it from `state.config.command_prefix`. Throughout this doc `/` means "the configured prefix". Resolved via the existing `Config::resolve` (defaults < file < CLI); no CLI flag needed.

## Command set — hybrid

`slash.rs` owns a **curated table** of friendly/parameterized commands; anything not in it falls back to the existing `Command` registry by kebab name.

### Curated table (v1)

| Slash | Args | Action / effect |
|------|------|-----------------|
| `/save [name]` | optional name | quick-save, or save-as `name` |
| `/load [name]` | optional name | restore (latest, or named slot) |
| `/reset [map]` | optional literal `map` | reset game; with `map`, also reset the map (#49) |
| `/panh <n>` | i32 | `Action::Pan(n, 0)` |
| `/panv <n>` | i32 | `Action::Pan(0, n)` |
| `/zoom <in\|out\|reset\|N>` | enum/i32 | `ZoomIn`/`ZoomOut`/`ZoomReset` (N = repeat in/out N times) |
| `/center` | — | `Action::Recenter` |
| `/tidy` | — | `Action::Retidy` |
| `/layer <next\|prev\|N>` | — | `CycleLayer(±1)` or jump to layer N |
| `/quit` | — | `Action::Quit` |
| `/help` | — | print the command list to the transcript (see below) |

Aliases (e.g. `/q`→`/quit`, `/w`→`/save`) live in the same table.

### Fallback

If the first token isn't a curated name, look it up via `keymap::Command::from_name(token)` (the kebab `name()` set) and dispatch its `to_action()` (no args). So `/open-config`, `/toggle-inventory`, `/cycle-layout`, `/retidy`, `/zoom-reset`, etc. all work. Curated entries WIN over the fallback when names collide (the curated `/zoom` shadows nothing; e.g. curated `/center` vs fallback `/recenter` both work).

### Parser

`slash::parse(input: &str) -> SlashOutcome` where `SlashOutcome = Action(Action) | Message(String /*status text*/) | Error(String)`:
1. Strip leading `/`, whitespace-tokenize.
2. Empty (`/` alone) → `Error("type /help for commands")`.
3. Match token0 against the curated table → run its `builder(args) -> Result<Action|Message, String>` (numeric parse via `str::parse::<i32>()`; bad/missing arg → `Error("usage: /panh <n>")`).
4. Else `Command::from_name(token0)` → `Action(cmd.to_action())`.
5. Else `Error("unknown command: /<token0> — try /help")`.
`/help` and `/save`/`/load`/`/reset` need app-level I/O, so the builder may return a `Message` or a sentinel the run loop handles (mirroring how `SubmitCommand`/`SavesSaveAs`/`ResetGame` are caller-handled today).

## Feedback — quiet

- **Visible-effect commands** (pan/zoom/center/tidy/layer) just dispatch their `Action`; no transcript or status text (the map moves — that IS the feedback).
- **Stateful commands** (save/load/reset) and **all errors/usage** set a transient **status message** shown on a status line (NOT the story transcript): e.g. `saved "slot1"`, `loaded`, `reset (map kept)`, `unknown command: /foo — try /help`.
- New `AppState.status_msg: Option<String>` (+ optionally a set-time for auto-expiry); rendered on a status line in `render/transcript.rs` (reuse/extend the existing status-bar area). Cleared on the next keypress/turn.

## `/help`

Prints the available slash commands to the **transcript** (explicit request, one-time — not spam): the curated names with arg hints (`/panh <n>`, `/zoom <in|out|reset|N>`, …) and a closing note that "any command name also works, e.g. /open-config". These transcript lines are tagged **Meta** (see below).

## Autocomplete — `/`-mode

`recompute_suggestions` (input.rs ~1703) currently completes from `state.dict_words` + room nouns. Add: if `state.input.starts_with('/')`, complete the FIRST token from the **slash name set** (curated names ∪ `ALL_COMMANDS` kebab names) by prefix instead, storing them in `state.suggestions`. Tab cycles them (existing `Action::Autocomplete` path). Once the name is complete and the command takes args, the suggestion line shows the **arg hint** (e.g. `/panh <n>`) rather than name completions. Reuses the existing suggestion-line rendering (`render/transcript.rs`).

## Transcript entry categories (forward-compat)

To support the future transcript filter (story/meta/both), give each transcript entry a category. Minimal v1: introduce `enum TranscriptKind { Story, Meta }` and tag pushes — game output (`session.submit` result, `> cmd` echoes) = `Story`; `/help` output and any app/slash messages pushed to the transcript = `Meta`. The filter UI itself is the separate console-search/export item; this spec only adds the tagging so slash output is categorizable. (If the transcript currently stores plain strings, wrap them in a small struct or a parallel kind vec — keep it minimal.)

## Components

- **`crates/app/src/slash.rs` (new):** the curated table, `parse(input) -> SlashOutcome`, the slash-name set (for autocomplete), and the `/help` text. Pure + unit-testable.
- **`main.rs`:** intercept `/` at submit; dispatch the `Action`/handle the caller-level commands (save/load/reset/help); set `status_msg`.
- **`config.rs`:** `command_prefix: char` (default `'/'`), read in `Config::resolve`.
- **`state.rs`:** `status_msg`, the transcript-entry `TranscriptKind` tagging, the slash-name set accessor.
- **`input.rs`/`complete.rs`:** `/`-mode in `recompute_suggestions`.
- **`render/transcript.rs`:** render `status_msg` on a status line; tag `/help`/meta pushes.

## Testing

- `parse`: `/panh -1`→`Pan(-1,0)`; `/zoom in`→`ZoomIn`, `/zoom 3`→3× zoom-in (or a `ZoomBy`), `/zoom reset`→`ZoomReset`; `/save foo`→save-as `foo`; `/reset map`→reset+map; `/save` (no name)→quick-save; `/open-config` (fallback)→`OpenConfig` action; `/panh` (missing arg)→`Error(usage…)`; `/foo`→`Error(unknown…)`; `/` alone→help hint.
- Fallback precedence: a curated name shadows the kebab fallback when they'd collide.
- Autocomplete: input `/pa` → suggestions include `panh`,`panv` (+ any `pan-*` fallback names); a non-slash input still completes from the dictionary (unchanged).
- Routing: a `/`-prefixed submit does NOT call `session.submit`, does NOT increment `turns`, and does NOT push a `> cmd` story line.
- Configurable prefix: with `command_prefix = ';'`, a `;help` input routes as a command and a `/help` input goes to the game (and vice-versa for the default); the parser/autocomplete key off the configured char.
- Feedback: a stateful command sets `status_msg`; an error sets `status_msg`; a pan command sets neither.
- `/help`: pushes the command list to the transcript tagged `Meta`.

## Out of scope / non-goals

- Interactive multi-step argument prompts (args are inline only; no follow-up PromptKind).
- The transcript FILTER UI and console search/export (separate item) — this spec only adds the `TranscriptKind` tag.
- Replacing or changing any keybindings (slash is an additional input path).
- `mapper`/`zvm` changes.

## Risks & limitations (accepted)

- **Name collisions** curated-vs-fallback resolved by curated-wins (documented); the fallback still reaches the kebab name.
- **`/` as the first game character:** IF games essentially never take a command starting with `/`, so intercepting it is safe — and if one does, the user sets a different `command_prefix`. A literal leading-prefix command can't reach the game while that prefix is configured (acceptable; change the prefix).
- **Status-line real estate:** the transient `status_msg` shares the status-bar row; it overwrites the status content briefly then clears — acceptable.
