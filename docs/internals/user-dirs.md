# Where lanthorn keeps the user's files

Design for SQ-1721, SQ-1722 and SQ-1723, settled with the user on 2026-10-06.
Lane A builds the roots (SQ-1721 + SQ-1722); lane B builds the shared IFDB
store on top of them (SQ-1723).

## The four roots

Everything lanthorn writes for a user hangs off four roots, carried together
as one value, `UserDirs { config, data, cache, logs }` — the same "facts that travel
together" rule as `DataRoots` and `MachineBoot`. `DataRoots` (catalogue /
player / documents / cache) is derived from a `UserDirs`, never from a bare
path.

| root | holds |
|---|---|
| **config** | `config.toml`, `style.toml` |
| **data** | `saves/` (the catalogue and the default player), `users/<name>/`, `documents/`, the user's system disks (`system_fonts::user_media_dir`) |
| **logs** (SQ-1726) | `crash.log`, `trace.log`, `stderr.log`, `dump-windows.log`, `dump-cells.log`, `dump-terminal.log`. Written through `Config::logs_root()`, which creates the folder on write. |
| **cache** | only what can be regenerated without network traffic or user action (today: `miss_cache`). Fetched IFDB records and covers are **data**, not cache: the OS and users purge caches, and refetching means unprompted IFDB traffic. |

### Platform defaults (new installs only)

| | config | data | cache | logs |
|---|---|---|---|---|
| macOS, Linux / other Unix | `$XDG_CONFIG_HOME/lanthorn` (`~/.config/lanthorn`) | `$XDG_DATA_HOME/lanthorn` (`~/.local/share/lanthorn`) | `$XDG_CACHE_HOME/lanthorn` (`~/.cache/lanthorn`) | `$XDG_STATE_HOME/lanthorn` (`~/.local/state/lanthorn`) |
| Windows | `%APPDATA%\lanthorn` | same as config | `%LOCALAPPDATA%\lanthorn` | `%LOCALAPPDATA%\lanthorn\logs` |

macOS takes the XDG layout rather than `~/Library`: lanthorn is a terminal app,
and terminal tools on macOS (git, gh, neovim, helix, Ghostty, kitty) keep their
files in `~/.config`, so they sync with dotfiles across Mac and Linux. An XDG
value counts only if it starts with `/`. A sandboxed Mac App Store build gets its
container through the explicit `UserDirs` an embedding host hands in, so the
defaults need not suit it. The single-folder layouts (`~/.lanthorn`,
`--user-dir`, Docker) keep logs at the base folder itself, so log files stay
where they always were.

The folder name is plain `lanthorn` (no reverse-DNS bundle id). Resolved with
the `dirs` crate's base directories plus our own name — not `ProjectDirs`,
whose names differ per platform. Linux deliberately splits config from data:
that is what XDG asks for, and a user who wants one folder keeps (or creates)
`~/.lanthorn`.

### Precedence

1. **Explicit roots** win: a `UserDirs` handed in by an embedding host
   through the library API (mobile containers, store builds with their own
   sync), or `--user-dir` / `--data-dir` on the command line. `--user-dir X`
   keeps meaning today's single-folder layout under X (config, `saves/`,
   `documents/`, `cache/` all inside X). `--data-dir` keeps standing in for the
   catalogue base, as `DataRoots::resolve` documents.
2. **Legacy**: if `~/.lanthorn` holds real content, it is used whole, exactly as
   today (single-folder layout, cache inside it). Real content
   (`user_dirs::has_real_content`) is a `config.toml`, a `style.toml`, a
   `saves/` with at least one file anywhere under it, or a non-empty `users/`;
   anything less (an empty `saves/`) is ignored so it cannot hide the platform
   folders' settings (SQ-1776). Nothing is migrated on its own.
   **When the platform folders hold real content too**, `UserDirs::conflict`
   (public, so a host can ask its own way) reports both sets, and
   `user_dir_conflict::rename_plan(&conflict, keep, date)` / `apply_rename_plan`
   (public, the same two the TUI prompt calls) give it the rename step: the
   `(from, to)` list for the set not kept, then the renames with per-move
   failures reported and nothing deleted or overwritten. The TUI asks in
   the plain terminal before the config is loaded (`user_dir_conflict.rs`): both
   sets with their `config.toml` mtime and save-file count, then an offer to
   rename the other set to `<name>.unused-YYYY-MM-DD` (`-2`, `-3` on a name
   clash; each platform folder separately; nothing deleted). Declining the
   rename applies the choice to this launch only (`pin_choice`). With stdin or
   stdout not a terminal it never reads stdin: the platform folders win and one
   line on stderr names both. `--user-dir` bypasses it. The one way out is opt-in: `lanthorn --migrate-user-dir
   [--yes]` (`migrate_user_dir.rs`) lists every move, asks, then moves
   `config.toml`/`style.toml` to the platform config root, the contents of
   `cache/` to the cache root, top-level `*.log` files to the logs root and
   everything else to the data root, deletes OS
   clutter (`.DS_Store`, `Thumbs.db`, `desktop.ini`), removes the emptied
   `~/.lanthorn`, and exits without starting the TUI. It refuses, changing
   nothing, if `~/.lanthorn` is missing, if `--user-dir` is also given, or if
   any destination already exists (no merging). A failure partway stops and
   reports what moved; nothing is rolled back. Entries are only renamed, never
   copied or deleted: if the OS refuses a rename (a different disk), the message
   names that item and asks the person to move it by hand.
3. **Platform defaults** above.

The TUI and every embedding host go through the same resolver, so on one
machine they share config and saves.

The Docker image passes `--user-dir /data/.lanthorn` explicitly, so container
volumes keep the single-folder layout `docs/internals/docker.md` describes.

### The home directory (SQ-1721)

There is exactly one "home directory" helper, built on `dirs::home_dir()`
(on Windows that is the profile from the Known Folder API, so an unset `HOME`
no longer matters). Every former `std::env::var("HOME")` in production code —
`config.rs` `default_user_dir`, `system_fonts.rs` `user_media_dir`,
`colors.rs` `expand_path` (`~` expansion), `main.rs` — goes through it or
through `UserDirs`. Nothing falls back to `.` (the working directory).

## The shared IFDB store (SQ-1723)

A story's fetched IFDB record and cover are kept **once per IFDB entry**, not
once per copy:

```
<catalogue>/ifdb/<tuid>.json        the record (formerly <copy>.save/info.json)
<catalogue>/ifdb/<tuid>.<ext>       the fetched cover, in the format IFDB served
                                    (.png, .jpg, …; formerly <copy>.save/cover.png)
```

* Flat: an entry holds exactly two files, so no per-entry folder. `ifdb/`
  cannot collide with a story folder, which always ends in `.save`.
* The cover keeps its original bytes and takes the extension of its real
  format; a lookup tries the known image extensions.
* It lives in the **catalogue**, shared by every player (SQ-1676), as does each
  copy's IFDB link.
* Each copy's own `<copy>.save/` keeps only its link state — the tuid it is
  linked to, not-found, or a user's "Wrong game?" choice — plus saves, map,
  notes and per-game settings. Refreshing an entry updates every copy linked
  to it; relinking a copy just points it at another entry.
* A blorb's own frontispiece still outranks a fetched cover, as today.

### Adoption of existing per-copy files

An existing `<copy>.save/info.json` / `cover.png` is read as before. The first
time one is used (or on the next fetch), it is **moved** into `ifdb/` under the
copy's tuid, so copies cannot drift apart again. If the shared store already
holds that entry, the newer fetch wins, and the other copy's stale files are
removed. Nothing is refetched to do this.

A startup sweep (SQ-1724, `ifdb_store::adopt_catalogue`, on a background thread)
does the same for every `*.save` directory directly under the catalogue, without
checking the IFID. That reaches copies lazy adoption never will: those whose
stored IFID no longer matches what lanthorn computes, and those whose story is not
in the scanned library. Copies with no shareable tuid are left exactly as they
are. It is idempotent, so there is no marker file.

## Docs

README describes the released build: the new locations go in with
`*Next release:*` tags until the release that ships them. The guide, the
config template comments and `docs/internals/persistence.md` /
`saves.md` / `docker.md` are updated in the same lanes.
