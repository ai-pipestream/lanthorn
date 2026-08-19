# Story-Picker Info Side-Panel — Design

**Date:** 2026-07-01
**Status:** Approved design → implementation plan pending
**TODO:** APP Enhancements / Near Term line 7 ("story list page … side-panel that shows info about the highlighted story … For blorb files list the file-structure")

## Goal

Add a toggleable, animated **info side-panel** to the pre-game story picker (the
full-screen selection shown when a *directory* is passed to `lanthorn` at
launch). Pressing `i` or `Tab` slides in a right-hand panel describing the
highlighted story: header/filesystem metadata, blorb resource structure
(including a matching sibling `.blb`/`.blorb`), matching saved games, and a
best-effort feature badge line.

Independently of the panel, **every list row carries compact artifact badges**
— a story-type badge (Z-code / Glulx) plus single-letter flags for "a blorb
exists", "a save exists", and "a hint file exists" — so the user can glance at a
story's type and artifacts without opening the panel. These are cheap existence signals only (no
deep introspection).

## Scope

- **In scope:** the pre-game launch picker only (`run_story_picker` /
  `draw_story_picker` in `crates/app/src/main.rs`, backed by
  `crates/app/src/picker.rs`).
- **Out of scope (non-goals):**
  - Mid-game story switching (no session teardown/reload).
  - Treaty-of-Babel *iFiction* / XML metadata parsing.
  - Recursive directory scanning (scan stays top-level, as today).
  - Fixing large-directory / large-blorb scan speed (separate Engine TODO).

## Behavior

- The picker opens with the **full-width list**, exactly as today.
- `i` **or** `Tab` toggles the info panel. It **slides** in/out: the panel width
  eases from `0` to its full width using the existing `anim::Tween` driven by
  `config.animation` (`enabled` / `easing` / `scroll_ms`). When
  `animation.enabled` is false the toggle is instant.
- The panel content always reflects the **currently highlighted** row.
- **Not persisted.** The panel always starts closed each launch; there is **no
  new config key**. The open/closed state lives only for the session.
- **Narrow-terminal fallback:** the list keeps a minimum usable width
  (`LIST_MIN_W = 24` columns). If the terminal is too narrow to show both the
  list at its minimum and a usable panel (`PANEL_MIN_W = 28`), `i`/`Tab` is a
  no-op (the panel refuses to open) so the list is never squeezed unusable.
- Header gains an `[i: info]` hint; the footer hint gains `i/Tab: info`.

### Row badges (always visible, panel-independent)

Every row shows a trailing badge cluster: a **story-type badge** followed by
single-letter artifact flags, each separated by a space:

```
Zork I: The Great Underground…        Z B S H
Enchanter                             Z B H
A Mind Forever Voyaging               Z B
Curses                                Z S
Anchorhead                            G B S
```

- **Story-type badge (always shown):** the story's engine, version-agnostic —
  `Z` for Z-code, `G` for Glulx (from `StoryMeta.engine`). The **specific**
  Z-machine version (v3/v5/v8, Glulx `major.minor.subminor`) is **not** on the
  row; it lives in the info panel (`version` / "Z-code v3" line). A blorb wraps
  its inner engine's type (`.zblorb` → `Z`, `.gblorb` → `G`).
- **`B`** — a blorb exists: the story is itself a blorb (`self_blorb.is_some()`)
  **or** a same-stem sibling `.blb`/`.blorb`/`.zblorb` file exists (one `stat`).
- **`S`** — at least one save exists for this IFID (a filename beginning with
  the IFID in the saves dir).
- **`H`** — the hint index contains this IFID.
- **Absent artifacts are omitted** (no greyed placeholder), so a row shows only
  the letters for what it has. The type badge is always present.
- All badge text (type badge + artifact letters) shares **one** configurable
  style with both foreground and background colours — the `story_badge` selector.
- **Each badge glyph is configurable** via the existing `[symbols]` config
  section — new fields `badge_zcode` / `badge_glulx` (type badges, defaulting to
  ASCII `Z` / `G`) and `badge_blorb` / `badge_save` / `badge_hint` (artifact
  badges, defaulting to `B` / `S` / `H`), each a string. A user may set any glyph
  (e.g. a Unicode symbol); as with the existing map-glyph overrides, choosing a
  wide/multi-cell glyph is the user's responsibility.

The `B` badge deliberately uses a **cheap same-stem sibling `stat`**, not the
panel's full `resolve_sound_blorb` (which also does a directory-scan match). In
the rare case a directory-scan-matched blorb exists without a same-stem sibling,
the panel's Resources section may show chunks while the row lacks a `B`; this is
an accepted trade-off to keep the row pass cheap.

## Data model

### `StoryMeta` (eager — computed in `scan_stories`)

Everything here derives from bytes `scan_stories` **already reads** (it loads
each file to validate + compute the IFID) plus one `fs::metadata` call — no
extra per-file reads.

```rust
pub struct StoryMeta {
    pub size_bytes: u64,          // fs::metadata().len()
    pub modified: Option<String>, // fs mtime → "YYYY-MM-DD" (None if unavailable)
    pub engine: Engine,           // ZCode | Glulx (from LoadedStory)
    pub format: String,           // "Z-code", "Glulx", "Blorb (Z-code)", "Blorb (Glulx)"
    pub version: Option<String>,  // Z: header[0x00] (e.g. "3"); Glulx: version word (e.g. "3.1.2")
    pub serial: Option<String>,   // Z only: header 0x12..0x18 ASCII (compile date). None for Glulx.
    pub release: Option<u16>,     // Z only: big-endian word at header 0x02. None for Glulx.
    pub ifid: String,             // already computed in scan
    pub features: Features,       // eager bits (see Features)
    // Blorb chunks when the STORY FILE ITSELF is a blorb (already parsed in scan).
    // Sibling-.blb chunks are resolved lazily (see StoryAux) to avoid extra reads.
    pub self_blorb: Option<Vec<ChunkInfo>>,
}

pub struct ChunkInfo {
    pub usage: String,     // "Exec" | "Pict" | "Snd " | "Data" … (ResourceEntry.usage, ascii)
    pub number: u32,       // ResourceEntry.number
    pub chunk_type: String,// "ZCOD" | "GLUL" | "PNG " | "OGGV" | "MOD " … (ResourceEntry.chunk_type)
    pub len: usize,        // ResourceEntry.len (bytes)
}
```

`StoryEntry` (in `picker.rs`) gains a `pub meta: StoryMeta` field alongside the
existing `path` / `title` / `filename`.

The row's **story-type badge** is selected purely from `StoryMeta.engine`
(`ZCode` → the `badge_zcode` glyph, `Glulx` → `badge_glulx`). No per-row helper
is needed — the render maps `engine` to the configured glyph. `engine` comes
from actually parsing the file during the scan, so a blorb whose extension
(`.blorb`/`.zblorb`/`.gblorb`) hides its inner format still shows the correct
type. The **specific** version stays out of the row and is shown only in the
panel (`StoryMeta.version`).

### `RowBadges` (eager for **all** rows — computed once at picker start)

The panel's data is per-highlight, but the row badges must render for every
visible row, so they are computed once when the picker starts (after
`scan_stories`) into a `Vec<RowBadges>` parallel to the story list. Each field
is a cheap existence check — no archive reads, no blorb parsing.

```rust
pub struct RowBadges {
    pub blorb: bool, // self_blorb.is_some() OR same-stem .blb/.blorb/.zblorb sibling exists
    pub save: bool,  // saves dir has a filename starting with this IFID
    pub hint: bool,  // hint index contains this IFID
}

// Testable helper in picker.rs. `save_names` is the saves dir listing read ONCE;
// `hint_index` is the shared index loaded once at picker start.
pub fn compute_row_badges(
    entry: &StoryEntry,
    save_names: &std::collections::HashSet<String>,
    hint_index: &hints::HintIndex,
) -> RowBadges;
```

### `StoryAux` (lazy — computed on highlight, cached per row)

These touch **other** files/dirs, so they are deferred until a row is actually
viewed and then cached (keyed by story index) so re-highlighting is free.

```rust
pub struct StoryAux {
    // Associated blorb resources when the story is NOT itself a blorb but has a
    // matching sibling (.blb/.blorb) or dir-scan match. Carries the source path
    // so the panel can label which file the chunks came from.
    pub assoc_blorb: Option<(PathBuf, Vec<ChunkInfo>)>,
    pub saves: Vec<persist_files::SaveInfo>, // list_saves(save_dir, ifid)
    pub hints_available: bool,               // hint index has this IFID
}
```

Resolution:
- `assoc_blorb`: call `blorb::resolve_sound_blorb(&entry.path)`. If it resolves
  to a path **different from** `entry.path` (i.e. a sibling, not the story
  itself — the self-blorb case is already in `StoryMeta.self_blorb`), record its
  `resources()` as `ChunkInfo` + the source `PathBuf`. Otherwise `None`.
- `saves`: `persist_files::list_saves(&save_dir, &entry.meta.ifid)` where
  `save_dir = cfg.user_dir.join("saves")` (identical to the running game —
  `saves_dir()` / `archive_path()` in `main.rs`). See the perf note below.
- `hints_available`: load the hint index once
  (`hints::load_hint_index(&cfg.user_dir.join("hints"))`) and test
  `index.get(&entry.meta.ifid).is_some()`. (Index load is cheap and shared, done
  once when the picker starts, not per row.)

## Features (badge line)

Best-effort **static** signals only. Glulx-unknowable features are **omitted**,
never guessed.

```rust
pub struct Features {
    pub sound: bool,    // Z Flags2 bit 7  OR  any associated blorb has_sounds()
    pub graphics: bool, // Z Flags2 bit 3  OR  associated blorb has a "Pict" resource
    pub colour: Option<bool>, // Z: Some(Flags2 bit 6).  Glulx: None (runtime Glk → omit)
    pub hints: bool,    // from StoryAux.hints_available (folded in when aux resolves)
}
```

Z-machine header reads (`Memory`/raw bytes already in hand):
- **Flags2** = big-endian 16-bit word at header offset `0x10`.
  - bit 3 (`0x0008`) → pictures/graphics requested
  - bit 6 (`0x0040`) → colours requested
  - bit 7 (`0x0080`) → sound effects requested
- `version` = byte `0x00`; `release` = word at `0x02`; `serial` = 6 ASCII bytes
  at `0x12..0x18`.

Glulx header reads: `version` = 4-byte word at offset `0x04`, formatted
`major.minor.subminor`. No colour/timed static signal → `colour = None`.

Blorb-derived signals (from `self_blorb` eagerly, and folded in from
`assoc_blorb` when the aux resolves): `has_sounds()` for `sound`; presence of a
`Pict` usage entry for `graphics`. Because `sound`/`graphics` can gain a `true`
from the *lazily* resolved sibling blorb, the badge line is recomputed when the
aux for a row resolves (the eager `Features` is the lower bound; aux can only
turn badges on).

## Panel layout

Bordered, titled panel on the right, drawn with the existing `paneframe` border
helpers. Sections top-to-bottom, each omitted when it has no content:

```
┌─ Info ─────────────────────────┐
│ Zork I                         │   title (story title)
│ zork1.z3 · 92 KB · 2026-06-30  │   filename · size · modified
│ Z-code v3 · Release 88         │   format+version · release
│ Serial 840726                  │   serial (Z only)
│ IFID ZCODE-88-840726           │   ifid
│ Features: sound graphics hints │   badge line (present badges only)
│                                │
│ Resources (zork1.blb)          │   ← header names the source file
│  Exec #0 ZCOD  92 KB           │
│  Snd  #1 OGGV  210 KB          │
│  Pict #1 PNG   14 KB           │
│                                │
│ Saves (2)                      │
│  (default)  turn 412 · 06-30   │
│  before troll  turn 88 · 06-29 │
└────────────────────────────────┘
```

- The **Resources** section lists `self_blorb` chunks (header: the story
  filename) or, when the story is not itself a blorb, `assoc_blorb` chunks
  (header: the sibling file's name). If both are absent, the section is omitted.
- The **Saves** section lists `StoryAux.saves` (`(default)` first, then named
  slots) with `turn <turns> · <saved_at>`; omitted when empty.
- If content exceeds the panel height, the panel scrolls with the shared
  scrollbar helper (`render::scroll`). (Panel content is currently short; a
  simple clip-to-height with a scrollbar when overflowing is sufficient — no new
  scroll state machine.)

## Files & responsibilities

1. **`crates/app/src/picker.rs`**
   - Add `StoryMeta`, `ChunkInfo`, `Features`, `Engine` (or reuse an existing
     engine enum) and the `meta` field on `StoryEntry`.
   - Populate `StoryMeta` in `scan_stories` from the bytes already read: detect
     blorb container (`blorb::Blorb::is_blorb` / `parse` / `resources` /
     `has_sounds`), pick the exec image for header parsing, read Z/Glulx header
     fields, compute the eager `Features`.
   - Small pure header-parse helpers (Z version/serial/release/flags2; Glulx
     version) — unit-testable without the filesystem.
   - Add `resolve_aux(entry, save_dir, hint_index) -> StoryAux` (lazy path).
   - Add `RowBadges` and `compute_row_badges` (the eager row-badge helper). The
     story-type badge needs no helper — render maps `StoryMeta.engine` to the
     configured type glyph.

2. **`crates/app/src/main.rs`** (`run_story_picker` / `draw_story_picker`)
   - At picker start (after `scan_stories`): read the saves-dir filenames once
     into a `HashSet<String>`, load the hint index once, and build
     `Vec<RowBadges>` via `picker::compute_row_badges` for every row. (The hint
     index is the same one reused for lazy `StoryAux` resolution.)
   - `draw_story_picker` appends each row's badge cluster (the type glyph for
     `engine`, then the present artifact glyphs, space-separated) styled with
     `story_badge`. All five glyphs come from `config.symbols`
     (`badge_zcode`/`badge_glulx`/`badge_blorb`/`badge_save`/`badge_hint`); pass
     them (a small `BadgeGlyphs` borrow) into `draw_story_picker` alongside `cs`.
   - Panel state: `info_open: bool` + a slide `Tween` (+ `from`/`to` fraction
     to support reversing mid-slide) + an aux cache `Vec<Option<StoryAux>>`.
   - Handle `i` / `Tab`: toggle `info_open`, arm the slide tween (respecting
     `AnimationConfig`); no-op when the terminal is too narrow.
   - Loop tick: continue-render while the slide tween is active (mirror the
     existing `list.has_active_animation()` 16 ms tick).
   - On highlight change: if `info_open` and the selected row's aux is `None`,
     resolve + cache it.
   - `draw_story_picker`: split `area` by the eased panel width; give the list
     the remainder (full width when the fraction is 0); draw `draw_info_panel`
     for the selected entry when the panel width > 0.
   - New `draw_info_panel(meta, aux, area, cs, buf)`.
   - Resolve `save_dir` / `hint dir` from `cfg.user_dir` exactly as the game
     does.

3. **`crates/app/src/archive.rs`**
   - Add `pub fn read_archive_meta(path: &Path) -> io::Result<Meta>` that unzips
     **only** `meta.json` (name/turns/saved_at) — avoids `load_archive` reading
     the full map + save + transcript just to show a save summary.
   - `persist_files::list_saves` switches to `read_archive_meta` (behavior
     identical; only cheaper). Existing `list_saves` tests must still pass.

4. **`crates/app/src/colors.rs` + `crates/app/src/style.rs`** (styleable-UI rule
   — every new UI element must be themeable)
   - New selectors + `ColorScheme` fields:
     - `story_info` (composite: panel body + border)
     - `story_info:title`
     - `story_info:label`
     - `story_info:value`
     - `story_badge` (composite: **fg + bg**) — shared style for the row badge
       cluster (type badge + artifact letters)
   - Wire each in `style_for_selector`, add to `SELECTORS` + a `SELECTOR_GROUPS`
     entry, and give defaults in **both** `ColorScheme` constructors (the
     hard-coded `Default` and the theme/palette-based one).

5. **`crates/app/src/config.rs`** (`SymbolConfig`, the `[symbols]` section)
   - Add five `String` fields — `badge_zcode`, `badge_glulx`, `badge_blorb`,
     `badge_save`, `badge_hint` — each with a `#[serde(default = …)]` returning
     `"Z"` / `"G"` / `"B"` / `"S"` / `"H"`, and set the same defaults in
     `impl Default for SymbolConfig`. An absent `[symbols]` section (or absent
     field) yields today's ASCII glyphs (no-op), matching the section's existing
     "defaults match hardcoded glyphs" contract.

## Performance

- `scan_stories` does **no new file reads** — `StoryMeta` is built from bytes
  already loaded during the existing scan-and-validate pass.
- `StoryAux` (sibling blorb, saves, hints) resolves **lazily on first highlight**
  and is cached, so a directory of many stories does not open every archive or
  probe every sibling up front.
- `read_archive_meta` reads only `meta.json` from each save archive rather than
  the entire ZIP.
- The hint index is loaded once when the picker starts.
- **Row badges** cost one `readdir` of the saves dir (shared across all rows,
  no per-save archive reads), the shared hint-index lookup, and one same-stem
  sibling `stat` per row — no blorb parsing, computed once at picker start.

## Testing

- **`picker.rs`**
  - `scan_stories` populates `StoryMeta` for a synthetic v3 story: version,
    serial, release, size, engine, and the Flags2-derived `Features`.
  - Blorb self-structure: a synthetic blorb (reuse the `make_blorb`-style helper
    from `hints.rs` tests / `blorb` tests) yields `self_blorb` chunks and
    `format = "Blorb (Z-code)"`.
  - Pure header helpers: flags2 bit extraction (sound/graphics/colour), serial
    ASCII decode, release word decode — table-driven, no filesystem.
  - `compute_row_badges`: table-driven over synthetic entries — `blorb` true for
    a self-blorb and for a same-stem sibling; `save` true only when a
    matching-IFID filename is in the `save_names` set; `hint` true only when the
    hint index contains the IFID; all three false when nothing matches.
- **`main.rs` render**
  - `draw_info_panel` renders the expected labels/values + a features line + a
    resources listing for a `StoryMeta`/`StoryAux` fixture.
  - `draw_story_picker`: list is full width when `info_open` is false; list
    width shrinks and a panel border appears when the panel fraction is 1.0.
  - Row badges: a Z-code story with all three artifacts renders `Z B S H`; a
    Glulx story with only a save renders `G S` (type badge always present, absent
    artifact letters omitted); the cluster is drawn with the `story_badge` style.
  - Configured glyphs: with the `badge_*` fields overridden, the row uses the
    overridden glyphs (including type glyphs) instead of the ASCII defaults.
- **`config.rs`**
  - `SymbolConfig` default: `badge_zcode`/`badge_glulx`/`badge_blorb`/
    `badge_save`/`badge_hint` are `"Z"`/`"G"`/`"B"`/`"S"`/`"H"`; a `[symbols]`
    TOML overriding one field parses and yields the override while the others
    keep their defaults.
  - Narrow-terminal: at a width below `LIST_MIN_W + PANEL_MIN_W`, toggling leaves
    the list full width (panel refuses to open).
- **`archive.rs`**
  - `read_archive_meta` returns the same `Meta` as `load_archive(...).meta` for a
    round-tripped archive.
- **Slide**: fraction interpolation via `Tween` endpoints (0 and 1); reversing
  mid-slide starts from the current eased fraction (unit-level where practical).

## Constraints

- **Cross-platform:** no new dependencies. `blorb`, `zip`, and `toml_edit` are
  already app deps; VM crates stay zero-dep and are untouched.
- **Styleable UI:** the panel, all its text roles, and the row badge cluster are
  themeable via `style.toml` selectors (above). No hard-coded colours. The badge
  glyphs are configurable via `[symbols]`; their defaults are ASCII single
  letters, so a default install stays terminal-safe on every platform.
- **Surgical:** the existing story-picker list rendering, sort, and navigation
  are unchanged except for the area split and the new hints; the Z-machine/Glulx
  load paths are untouched.
