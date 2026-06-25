# Transcript Text Styling by Category — Design

**Date:** 2026-06-25
**Status:** Approved, ready for planning
**TODO:** #75 (the transcript-categories slice of the "UI styling expansion" theme; #76/#77/#78 input/score/autocomplete chrome and #82 per-side borders are separate sub-designs.)

## Goal

Make transcript text themeable by what it *is*, not one flat color. Two layers:

1. **Structural categories** (tagged where we emit the line): the player's
   **input** echo, **story** (game) output, **meta** (app/slash) output, and VM
   **warnings** — each with its own `fg/bg/bold/italic`.
2. **Story sub-styling** (recognized by content): built-in rules for the
   **room-name/location** line and **bracketed system** lines, plus
   user-defined `regex → style` rules in `style.toml`.

## Background (current state)

- `TranscriptKind` has two variants, `Story | Meta` (`state.rs`). Both render
  with the single `state.colors.transcript` style; Meta lines only get a
  left **gutter** (`▏`, a hardcoded `META_GUTTER` glyph styled by `meta_marker`)
  and wrap to `width - 2` (`render/transcript.rs`).
- The player's echoed command is pushed as **Story**; VM diagnostics (from the
  sound wave) are pushed as **Meta**. Neither is distinguishable today.
- `/filter story|meta|both` keys off `TranscriptKind` via
  `visible_transcript_indices`.
- The style system resolves `style.toml` selectors into a `ColorScheme`
  (`SELECTOR_FIELDS` / `apply_color_decls` / `write_style_full`); glyphs live in
  `SymbolSet`. `regex` is **not** yet a dependency.
- `AppState` has no current-room *name* (only `RoomId`s); `TurnResult.location`
  carries the name each turn.

## Design

### 1. Structural categories (data model)

Expand the enum:

```rust
pub enum TranscriptKind { Story, Input, Meta, Warning }
```

Tag at the emit sites:
- **Input** — the echoed player command (today pushed as Story).
- **Warning** — VM diagnostics (today pushed as Meta in `apply_turn_events`).
- **Meta** — app/slash output (`/help`, status messages) — unchanged.
- **Story** — game output — unchanged.

**Filter mapping** (keep `TranscriptFilter { Story, Meta, Both }` — the filter
stays coarse, the styling is finer): in `visible_transcript_indices`,
`Story` matches kind ∈ {Story, Input}; `Meta` matches kind ∈ {Meta, Warning};
`Both` matches all. (So "story" = the game conversation incl. your commands;
"meta" = app + engine chatter.)

### 2. Per-category text styling + gutters

New `ColorScheme` `Style` fields and selectors (Story keeps `transcript`):

| Selector              | Applies to            |
|-----------------------|-----------------------|
| `transcript`          | Story text (existing) |
| `transcript:input`    | Input (echoed command)|
| `transcript:meta`     | Meta text             |
| `transcript:warning`  | Warning text          |

**Gutters — individually configurable for Meta and Warning** (glyph *and*
color separate, per the codebase's symbol/color split):
- Glyphs: two new `SymbolSet` slots **`meta_gutter`** (default `▏`) and
  **`warning_gutter`** (default `!`).
- Colors: `meta_marker` (existing) styles the meta gutter; new
  **`warning_marker`** selector styles the warning gutter.
- Both Meta and Warning lines reserve the 2-col gutter and indent text past it
  (today's Meta behavior, now also for Warning). Story and Input get no gutter.
- **Wrapping honors the gutter.** A gutter line wraps its text to `width - 2`
  and indents **every** visual row by the 2-col gutter, not just the first.
  The gutter glyph is drawn once on the first row; continuation rows show 2
  blank gutter columns so wrapped text stays in its column and never renders
  under (or to the left of) the gutter. Today's code wraps Meta to `width - 2`
  but does not re-indent continuation rows — this is the bug being fixed.

### 3. Story sub-styling rules

Each line tagged **Story** is matched against an ordered rule list; the
**first** matching rule's style is **patched over** the base `transcript` style
(a rule overrides only the properties it sets). Evaluation order:

1. **User rules** (in `style.toml` order) — so users can override built-ins.
2. **Built-in rules.**
3. No match → bare `transcript`.

Matching is **whole-line** (one resolved style per line; per-substring inline
highlighting is out of scope — see below).

**Built-in rules:**
- **location** → `transcript:location`. Matches a Story line whose normalized
  text equals, or is a word-boundary leading-prefix match against,
  `AppState.current_room_name` (the live detected room name). Reuses the same
  match shape as v4 detection (`status_name_matches`): equality or `room_name`
  is a leading prefix of the line ending on a non-alphanumeric boundary.
- **system** → `transcript:system`. Matches a Story line whose trimmed text is
  fully bracketed: `^\[.*\]$` (e.g. `[Your score just went up by ten points.]`).

New selectors `transcript:location`, `transcript:system`.

**User rules** — a new ordered array in `style.toml`:

```toml
[[transcript.rule]]
match = "^>.*"          # a regex
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"
fg = "red"
```

Each rule is `{ match: String (regex), fg?, bg?, bold?, italic? }`. Rules are
**compiled once at style load** into `Vec<CompiledRule { regex: Regex, style:
Style }>` stored on the `ColorScheme`. Adds the **`regex` crate** dependency to
`crates/app`. An invalid regex is dropped with a load **warning** (consistent
with how unknown selectors warn), never a panic.

**Current room name source:** add `AppState.current_room_name: Option<String>`,
updated each turn from `TurnResult.location` (set in `apply_turn_events`
alongside `loc_method`). The location rule no-ops when it is `None`.

### 4. Defaults

Restrained but distinct (`terminal_default`; `from_ghostty` uses palette
equivalents):
- `transcript` (Story): unchanged.
- `transcript:input`: `fg = Cyan`.
- `transcript:meta`: `fg = DarkGray` (dims meta text — a small intended change
  from today's plain white).
- `transcript:warning`: `fg = Yellow`.
- `transcript:location`: `bold` (inherits Story fg, bold header).
- `transcript:system`: `fg = DarkGray`.
- `meta_marker`: `fg = DarkGray` (existing); `warning_marker`: `fg = Yellow`.
- No user `[[transcript.rule]]` entries by default.

## Architecture / components

- `crates/app/src/state.rs` — `TranscriptKind` enum (+2 variants);
  `current_room_name` field + default; filter-mapping update in
  `visible_transcript_indices`.
- `crates/app/src/session.rs` / `main.rs` — tag the echoed command as Input;
  tag diagnostics as Warning (in `apply_turn_events`); set `current_room_name`.
- `crates/app/src/colors.rs` — new `Style` fields (`transcript_input`,
  `transcript_meta`, `transcript_warning`, `transcript_location`,
  `transcript_system`, `warning_marker`) + a `transcript_rules:
  Vec<CompiledRule>` field; defaults in both constructors.
- `crates/app/src/style.rs` — the five new selectors in `SELECTOR_FIELDS` /
  apply / export; parse the `[[transcript.rule]]` array, compile regexes (warn +
  skip on error), and resolve the built-in `location`/`system` selectors.
- `crates/app/src/symbols.rs` — `meta_gutter` / `warning_gutter` glyph slots
  (defaults `▏` / `!`) with overrides + export.
- `crates/app/src/render/transcript.rs` — resolve each line's style by kind,
  then (for Story) apply the rule list (user → built-in → base, first-match
  patch); per-category gutter glyph/style for Meta and Warning. Wrap gutter
  lines to `width - 2` and indent **every** wrapped row past the 2-col gutter
  (glyph on row 1, blank gutter columns on continuation rows).
- `crates/app/Cargo.toml` — add `regex`.

## Error handling

- Invalid user regex → load-time warning, rule skipped, rest load.
- `current_room_name == None` → location rule simply doesn't match.
- Missing/short `transcript_kinds` entry defaults to `Story` (today's behavior).

## Testing

- `TranscriptKind`: input echo tagged Input; diagnostics tagged Warning;
  game output Story; app output Meta.
- Filter mapping: `story` shows Story+Input, hides Meta+Warning; `meta` shows
  Meta+Warning, hides Story+Input; `both` shows all.
- Rule resolution: user rule beats built-in; first-match wins; patch semantics
  (a rule setting only `bold` keeps the base fg); no match → base `transcript`.
- Built-in location: a Story line equal to / leading-prefix of
  `current_room_name` → location style; boundary guard ("Hall" line vs room
  "Hallway" does not match); `None` room name → no match.
- Built-in system: `[bracketed]` line → system style; non-bracketed → not.
- Regex: valid pattern compiles and matches; invalid pattern → one warning,
  skipped, other rules still load.
- Selectors parse/apply/export (incl. `write_style_full` round-trip); both
  gutter glyphs + `warning_marker` resolve.
- Render: each kind draws with its style; Meta/Warning draw their own gutter
  glyph+color; a rule-matched Story line draws the patched style.
- Wrapping: a Meta/Warning line long enough to wrap indents every continuation
  row past the 2-col gutter (no wrapped text in the gutter columns); the gutter
  glyph appears only on the first row.

## Out of scope (deferred)

- **Per-substring inline highlighting** (recolor just the matched word within a
  line) — whole-line only for now.
- Rules applying to non-Story categories (Input/Meta/Warning already have their
  own styles).
- Additional built-in detectors beyond location/system (users can add regex
  rules).
- #76 score-bar, #77 input-line/prompt/autocomplete, #82 per-side borders —
  separate sub-designs in the styling theme.
