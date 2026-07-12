# SQ-0284 — Story-filename storage key + flat per-game directory + `--data-dir` — Design

**Status:** design (awaiting user review)
**Date:** 2026-07-12
**Depends on:** SQ-0283 (shares the CLI + `persist_files` code; stacked on `sq-0283-unified-saves`)
**Blocks:** SQ-0285 (browser save listing reads the per-game dir this defines)

## Problem

Storage naming and location are inconsistent across the three hosts (verified against the code):

| Host | Key today | Base dir today | Files |
|------|-----------|----------------|-------|
| zvm-cli | IFID (`compute_ifid`) | **story dir** | `<ifid>.aux` (magic `ZAUX`) |
| gvm-cli | story filename | **story dir** | `<story>.glkvfs` |
| app | IFID | **`~/.babelmap/saves`** | `<ifid>.babelmap`, `<ifid>.aux` (magic `ZAX1`), `<ifid>.gvfs`, `<ifid>-<slug>.babelmap`, `<ifid>-<slug>.qzl` |

Three problems:
1. **Different keys** — IFID vs filename; the app's Glulx IFID is the Z-machine `ZCODE` formula *misapplied* to Glulx header bytes (`compute_ifid` at `main.rs:1840` / `picker.rs:465` has no engine branch).
2. **Different default locations** — CLIs write next to the story; the app centralizes in `~/.babelmap/saves`.
3. **No configurability** — nothing lets a user redirect where saves/sidecars live.

## Goals

1. **Storage key = the story's filename**, across all three hosts. Drop IFID from *storage* keying entirely.
2. **Flat per-game directory** `<base>/<story-filename>/`, files told apart by extension (`.aux`, `.glkvfs`, `.qzl`, `.babelmap`).
3. **`--data-dir <path>`** on all three hosts overrides the base.
4. **Default base:** app = `~/.babelmap/saves` (unchanged home; **user's explicit choice** to keep the app central); CLIs = the story's own directory. Consistency is *structural* (same layout + key + override), defaults intentionally differ.
5. **IFID computation stays** for its real jobs — known-title lookup (`session::known_title`) and hint association (`hints::HintIndex`). Untouched.
6. **No migration** (alpha). Existing `<ifid>.*` files orphan; note in the changelog/docs.

## Non-goals (explicitly out of scope)

- **Unifying the aux *format*.** zvm-cli writes magic `ZAUX`; the app writes `ZAX1`. They stay distinct. Consequence: if the same story is opened in *both* zvm-cli and the app against the *same* `--data-dir`, they land the same `save.aux` filename but can't read each other's — the reader rejects a foreign magic and simply ignores it (no corruption, confirmed: `aux_preload`/`read_global_aux` return `None` on bad magic). Documented as an alpha limitation; format unification is a separate quest if wanted.
- **Fixing the Glulx IFID misapplication** for title/hint lookups (storage no longer uses IFID, so this stops mattering for storage; the title/hint correctness for Glulx is a separate concern).
- **Blorb IFmd / content-hash key** — dropped in the simplified design.
- **Per-file suffix-override flag** — moot now that inner names are fixed.
- **Moving map exports** (`user_dir/maps/<ifid>.svg`) — not among the four file types; stays IFID-keyed under `user_dir/maps`.
- **The explicit-path *export* escape hatch** — when a user types a `@save`/`@restore` value that contains a path separator, it is honored verbatim (out to any path). This spec only wires the *bare-name → per-game dir* resolution; the broader "export to a chosen destination" UX (for saves, Save States, and maps) is deferred to **SQ-0288**.

## Design

### The key: `story_key(path)`

The per-game directory name is the story file's **basename including extension**, sanitized:

- `/games/Zork1.z5` → `Zork1.z5`
- `/games/Advent.gblorb` → `Advent.gblorb`

Sanitization reuses the existing charset (`[A-Za-z0-9._-]` kept, others → `_`, empty → `game`). **Including the extension** (not the stem) is deliberate: it distinguishes `Zork1.z5` from `Zork1.gblorb` and is the most literal reading of "story filename," at the cost of a dir named with a dot-extension (valid on all target filesystems).

> _Open choice for review:_ basename-with-extension (`Zork1.z5/`) vs stem-only (`Zork1/`). Recommending **with-extension** for collision-safety. Flag if you'd rather have bare stems.

The user owns collisions: two identically-named stories sharing one `--data-dir` share a folder (rename one, per the settled "keep it simple, user owns it" decision).

### Layout — flat per-game directory

```
<base>/<story-filename>/
    default.aux        # Z-machine aux sidecar    (singleton; was <ifid>.aux)
    default.glkvfs     # Glulx VFS sidecar         (singleton; was app <ifid>.gvfs / gvm-cli <story>.glkvfs)
    default.babelmap   # default Save State slot   (auto per-turn + Ctrl+S default; was <ifid>.babelmap)
    <slug>.babelmap    # named Save States         (was <ifid>-<slug>.babelmap)
    <slug>.qzl         # in-game @save files        (app named slugs + CLI interactive bare names; was <ifid>-<slug>.qzl)
```

- The three auto/singleton files share the fixed stem **`default`**. `default` is a **reserved slug** — the app rejects/suffixes a user-named save called `default` so it can't clobber the auto slot or a sidecar. (Extensions already separate `.aux`/`.glkvfs`/`.babelmap`, so `default.aux`, `default.glkvfs`, and `default.babelmap` coexist; only a user *Save State* named `default` would collide with the auto `default.babelmap`.)
- `.glkvfs` is now the single canonical Glulx-VFS extension everywhere — the app's `.gvfs` is renamed to `.glkvfs` (same bytes; `vfs_bytes()`/`load_vfs` format unchanged).
- Per-base default: `<base>` = `~/.babelmap/saves` (app) or the story dir (CLIs), overridable by `--data-dir`.

### `--data-dir` plumbing

- **app** (clap): add `--data-dir <PATH>` to `struct Cli` (`config.rs`), beside the existing `--user-dir`. When set, the save base becomes `<data-dir>`; else `~/.babelmap/saves`. (`--user-dir` still governs maps/hints/config home; `--data-dir` governs only the per-game save/sidecar base.)
- **zvm-cli / gvm-cli** (hand-rolled): add a `--data-dir <path>` arm to each `parse_args`/argv scan. When set, the sidecar base becomes `<data-dir>`; else the story's own directory.

### Per-host changes

**app** (largest surface):
- New `story_key(&Path) -> String` (sanitized basename) — replaces IFID as the *storage* key. Keep `compute_ifid` and `state.ifid` for titles/hints/display.
- Save base: `saves_dir(user_dir)` → `<user_dir>/saves` stays the *root*; the per-game dir is `<base>/<story_key>/`. Add `game_dir(base, key)` = `base.join(key)`; `create_dir_all` on first write.
- Rewrite the `<ifid>.*` path builders to `<game_dir>/default.*` and `<ifid>-<slug>.*` to `<game_dir>/<slug>.*`:
  `ifid::archive_path`, `aux_store::aux_path`, `vfs_store::vfs_path` (+ `.gvfs`→`.glkvfs`), `persist_files::{list_saves, save_named, save_game_named, save_game_named_bytes, list_qzl}` and the `main.rs` call sites threading `&ifid` for storage (§C.3 of the map).
- Picker badge (`compute_row_badges` / `resolve_aux`): switch save-presence detection from `save_names.starts_with(ifid)` to "does `<base>/<story_key>/` exist and contain a `.babelmap`/`.qzl`". Title/hint lookups still key off `entry.meta.ifid`.
- `--data-dir` threaded from `Cli` into the picker (`main.rs:1066`) and the game session (`main.rs:1842`) base.

**zvm-cli:**
- `aux::aux_path`: from `<story-dir>/<sanitized-ifid>.aux` to `<base>/<story_key>/default.aux`. `base` = `--data-dir` or story dir. `create_dir_all` on write.
- Interactive `@save`/`@restore` (see *Interactive save-name resolution* below).

**gvm-cli:**
- VFS path: from `<story>.glkvfs` to `<base>/<story_key>/default.glkvfs`. `base` = `--data-dir` or story dir. `create_dir_all` on write.
- Interactive `@save`/`@restore` (see *Interactive save-name resolution* below).

### Interactive save-name resolution (both CLIs)

The `@save`/`@restore` prompt value is interpreted:
- **bare name** (no path separator, e.g. `quicksave`) → `<base>/<story_key>/<name>.qzl` inside the managed per-game dir. `.qzl` is appended if absent. This is now the default, so CLI in-game saves populate the same per-game dir the app uses.
- **explicit path** (contains a separator or is absolute, e.g. `/tmp/foo.qzl`) → honored verbatim (escape hatch; the fuller export UX is SQ-0288).

`--restore`/restore listing resolves the same way. This makes the four file types uniform across all three hosts.

## Testing

- Unit: `story_key` sanitization + extension-preservation (`Zork1.z5` ≠ `Zork1.gblorb`, weird chars → `_`, empty → `game`).
- Unit: `game_dir` / each rewritten path builder returns `<base>/<key>/default.<ext>` and `<base>/<key>/<slug>.<ext>`; reserved-slug `default` rejected/suffixed.
- Unit: interactive save-name resolution — bare name → `<base>/<key>/<name>.qzl` (`.qzl` appended); value with a separator → verbatim.
- Unit (app): `list_saves`/`list_qzl` enumerate a per-game dir correctly (named slots, default slot, none).
- Unit (app): picker badge true iff a per-game dir has a save; false for an empty/absent dir; hint/title still keyed by IFID.
- Round-trip (app): write aux/vfs/Save State into `<tmp>/<key>/`, read back.
- CLI: `--data-dir <tmp>` redirects the sidecar + bare-name saves; default (no flag) writes the per-game dir next to the story.
- Regression: `compute_ifid`, `known_title`, and hint lookups unchanged.

## Rollout

No migration. Add a changelog/docs line: existing `<ifid>.*` saves/sidecars orphan under the new filename-keyed layout (alpha; Glulx persistence is ~3 weeks old, Z-machine users re-create or manually move). Update `docs/persistence.md` + `README` storage-location notes and the `--data-dir` flag docs.
