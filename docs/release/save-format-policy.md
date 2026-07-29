# Save-format policy (beta)

[← back to README](../../README.md) · see also [The persistence model](../persistence.md)

Until the first public beta, babelmap's rule was *"pre-release: formats may break
freely, no back-compat"*. The beta flips that for the formats that live on a
user's disk between sessions. Every persisted byte format is now (a) enumerated,
(b) version-stamped where it is a private babelmap format, and (c) pinned by a
round-trip freeze test, so **any change to a persisted format is deliberate** —
never an accident that silently corrupts a user's saves.

## The rule going forward

Changing the wire layout of any format in the table below requires, in the same
change:

1. **Bump its version marker** (the `*_VERSION` constant / `format_version`).
2. **Update the freeze test** that pins the constant (it will fail until you do —
   that is the point).
3. **Add a release-note entry** describing the break, plus a migration path
   (a tolerant reader for the old layout) *or* a documented, accepted break.

Pre-beta there is still **no obligation to read old files** (see the standing
"no back-compat before release" policy); the freeze machinery exists so that
*after* beta, breaks are conscious decisions with a paper trail — not surprises.

## Guarantee tiers

- **Public spec** — a standard interchange format defined outside babelmap. We
  read and write it to the published spec and it stays interoperable with other
  interpreters. We do not get to "version" it; identity/compatibility is the
  spec's own (e.g. Quetzal `IFhd` release/serial/checksum).
- **Frozen (0.x)** — a private babelmap format carrying a version marker, pinned
  by a freeze test. It may still break between 0.x versions, but only via the
  bump-and-note ritual above. A reader rejects a *newer* marker cleanly (empty /
  error, never a mis-parse).
- **Tolerant (unversioned by nature)** — TOML/JSON config and metadata. Missing
  fields default; unknown fields are ignored. Not byte-pinned; a `schema`/`format`
  integer guards the shape where one exists.

## Inventory

| Format | File / entry | Defined in | Version marker | Guarantee | Freeze test |
|---|---|---|---|---|---|
| Z-machine Quetzal (`@save`) | `game.qzl` inside `<slug>.babelmap` (app); bare `<slug>.qzl` (`zvm-cli`) | `zvm/src/quetzal.rs` | none — IFF `FORM IFZS`, identity via `IFhd` | Public spec (Quetzal 1.4) | `quetzal::tests::round_trip_restores_full_state`, `…rejects_serial_mismatch` |
| Glulx-Quetzal (`@save`) | `game.glksave` inside `<slug>.babelmap` (app); bare `<slug>.qzl` (`gvm-cli`) | `gvm/src/exec.rs` `save_quetzal` | none — spec-defined `FORM IFZS` | Public spec (Glulx §1.8) | `exec::tests::save_quetzal_is_a_wellformed_ifzs_container`, `…omits_greg_and_glk_chunks` |
| Host Save State — Z-machine | inside `.babelmap` `game.qzl` | `zvm/src/quetzal.rs` (+ archive) | via archive `format_version` | Frozen (0.x) | archive round-trip tests |
| Host Save State — Glulx | inside `.babelmap` `game.glksave` | `gvm/src/exec.rs` `save_state` (adds `GReg` + `Glk `) | `Glk ` chunk: `GLK_SNAPSHOT_VERSION = 6` | Frozen (0.x) | `glk::tests::snapshot_version_constant_is_frozen`, `…serialize_stamps_current_snapshot_version`, `…deserialize_rejects_future_snapshot_version`, `exec::tests::save_state_is_the_same_container_plus_our_own_chunks` |
| `.babelmap` archive (map + save + transcript + screen + history + pictures) | `<ifid>.babelmap` (ZIP) | `app/src/archive.rs` | `Meta.format_version = 5` | Frozen (0.x) | `archive::tests::format_version_constant_is_frozen`, `…unknown_format_version_returns_err`, `…save_trigger_wire_names_are_pinned_and_round_trip`, archive round-trip tests |
| Z-machine aux data (v5 `@save`/`@restore` table) | `default.aux` | `app/src/aux_store.rs` + `zvm-cli/src/auxiliary.rs` | `ZAUX` magic + `VERSION = 1` | Frozen (0.x), cross-host | `aux_store::tests::version_constant_is_frozen`, `…decode_rejects_bumped_version`, `…encodes_canonical_zaux_bytes` |
| Glk file VFS sidecar | `default.glkvfs` | `gvm/src/glk.rs` `encode_files`/`decode_files` (path: `app/src/vfs_store.rs`) | `GVFS` magic + `u32` version `1` | Frozen (0.x) | `glk::tests::encode_files_roundtrips_and_skips_temp`, `…decode_files_rejects_bumped_gvfs_version` |
| Debug-coverage PC set | `default.pcs` | `app/src/pcset_store.rs` | `ZPCS` magic + `VERSION = 1` | Frozen (0.x) | `pcset_store::tests::version_constant_is_frozen`, `…decode_rejects_bumped_version`, `…codec_round_trips` |
| Map graph | `map.json` (inside `.babelmap`) | `mapper/src/persist.rs` | JSON `version: 1` field | Tolerant (JSON) — carried by the archive | `mapper::persist::tests` round-trips |
| Per-story metadata | `info.json` (+ cover) | `app/src/story_info.rs`, `fetch_worker.rs` | JSON `format_version = 1`, `fetch_version = 1` | Tolerant (JSON) | `story_info::tests` |
| Global config | `config.toml` | `app/src/config.rs` | TOML `version` (`CONFIG_SCHEMA_VERSION = 1`) | Tolerant (TOML) | `config::tests` |
| Theme / per-game config | `style.toml`, `<ifid>.config.toml` | `app/src/config.rs`, `styles.rs` | none (TOML, field-tolerant) | Tolerant (TOML) | — |

## Version history

- **`.babelmap` archive 4 → 5 (SQ-0531).** `meta.json` gained
  `trigger: "ingame" | "hoststate"`, recording whether the game's own `@save` or
  the host's Save State wrote the archive — and therefore which PC convention the
  `game.<ext>` bytes inside follow. Restore dispatches on it instead of on the
  file extension, because `@save` now writes an archive too.
  *Accepted break, no migration:* a pre-5 archive still loads (the field defaults
  to `"hoststate"`, which is what every archive written before the bump actually
  was), but a v5 archive is rejected by older builds, as the freeze machinery
  intends. Bare `.qzl`/`.glksave` interchange files are untouched.

## Notes on identity vs. version

Quetzal and Glulx-Quetzal carry **no babelmap version** — they are public
interchange formats. Their safety net is the `IFhd` identity chunk (story
release + serial + checksum): restoring a save into the wrong story is rejected
(`ZError::SaveMismatch` / `GError::BadSave`), which is the standard's own
compatibility mechanism. We deliberately keep these formats spec-clean so other
interpreters can read our `@save` files and vice-versa.

The three private binary sidecars (`ZAUX`, `ZPCS`, `GVFS`) and the two versioned
containers (the `.babelmap` archive, the `Glk ` snapshot chunk) all reject a
**newer** version marker cleanly — an empty table/set/map, or a clean error —
rather than mis-parsing future bytes as the current layout. That reject behavior
is itself pinned by a freeze test, so a future bump has to consciously decide how
old readers see the new file.
