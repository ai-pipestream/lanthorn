# Configurable Interpreter Number — Design

**Date:** 2026-06-30
**Status:** Approved (design)

## Problem

We advertise interpreter number **6 (IBM PC)** to every game (`crates/zvm/src/screen.rs:346`). BeyondZork's IBM-PC path is monochrome: on interpreter 6 it only ever calls `set_colour(1,1)` (default/default) and never uses real colours, so its menus fall back to reverse video. The colour pipeline itself is correct — verified by advertising a colour-capable interpreter, under which BeyondZork emits real colour SGR (`ESC[37;40m`, `ESC[36;40m`, `ESC[3;37;40m`). The interpreter number is the sole gate.

Reference behaviour (Frotz `src/curses/ux_init.c`): the interpreter number defaults to **1 (DECSystem-20)** for V1–5 games and **6 (IBM PC)** only for V6. Frotz therefore gives BeyondZork colour; we do not.

## Goal

1. Default the interpreter number using Frotz's exact rule: `version == 6 → 6`, else `1`. Since we reject V6 (`ZError::GraphicalV6`), every game we load defaults to **1 (DEC-20)**.
2. Allow an explicit override via app config (`config.toml`) and a zvm-cli flag (`-I N` / `--interpreter N`, matching dfrotz).

## Non-Goals

- No in-app config-screen row (config.toml + CLI flag only).
- No per-title heuristic beyond the version rule (Frotz's per-title V6 logic is moot — V6 is rejected).
- No change to the colour rendering pipeline (already correct).

## Character-Graphics Note (why this is safe)

The engine renders high-byte characters two ways, both already supported:

- **Interpreter 6 (IBM PC):** `exec.rs:1582` gates a CP437 passthrough (`interp_ibm_pc = read_byte(0x1E) == 6`) so bytes 128–255 render as CP437 box-drawing.
- **Non-IBM interpreters:** BeyondZork uses the **Font-3 path** (`set_font 3`), rendered via `font3_translate` (confirmed by the existing test `print_char_not_cp437_under_other_interpreter`, `exec.rs:5076`, which notes Amiga "takes the Font 3 path, not CP437").

So under the new default (1) BeyondZork draws its map box via Font-3 *and* gains colour. The CP437 passthrough stays gated at `== 6` and remains available to anyone who sets interpreter 6 via the override.

## Design

### Component 1 — Engine (`zvm`)

- Add `Machine.interpreter_number: Option<u8>` (`None` = auto). Default `None` in both constructors.
- Add `pub fn set_interpreter_number(&mut self, n: Option<u8>)` — stores the field (applied at `init_caps`).
- Add a small helper `fn default_interpreter_number(version: u8) -> u8 { if version == 6 { 6 } else { 1 } }` (Frotz's rule).
- `Machine::init_caps` passes the override into `init_header_caps`, which writes `0x1E`:
  `let num = override.unwrap_or(default_interpreter_number(version)); mem.write_byte(0x1E, num);`
  (`init_header_caps` gains an `interpreter_number: Option<u8>` parameter, mirroring the existing `honor_game_colours` parameter.)
- The CP437 gate at `exec.rs:1582` is unchanged (`== 6`).

Effect ordering: hosts call `set_interpreter_number` **before** `init_caps` (same discipline as `set_honor_game_colours`).

### Component 2 — App (`app`)

- Add a config field `interpreter_number: Option<u8>` (serde, default `None` = auto). Mirror how `honor_game_colours` is defined/serialized in the config.
- Thread it into `GameSession::new`, which already runs `init_caps` internally: add an `interpreter_number: Option<u8>` parameter and call `machine.set_interpreter_number(interpreter_number)` before `init_caps`, next to the existing `set_honor_game_colours` call.
- Update `GameSession::new` call sites to pass the config value (or `None`).

### Component 3 — zvm-cli

- Parse `-I N` / `--interpreter N` into `Option<u8>` (absent = `None`). Reuse the argument-filtering approach already used for `--no-game-colours` so the value and flag are removed from the positional args (story path).
- Thread into `build_machine`: add an `interpreter_number: Option<u8>` parameter; call `machine.set_interpreter_number(n)` before `machine.init_caps()`.
- Apply on restart too if the CLI re-inits caps on restart (mirror where `set_honor_game_colours` is re-applied).

## Data Flow

```
config.toml interpreter_number ─┐
   -I / --interpreter N ────────┼─► Option<u8> ─► set_interpreter_number(n)
                                │                        │ (before init_caps)
   (absent) ────► None ─────────┘                        ▼
                                          init_header_caps writes 0x1E =
                                          n.unwrap_or(version==6 ? 6 : 1)
```

## Error Handling

- Out-of-range / non-numeric `-I` value: treat as absent (fall back to auto) — do not abort. (zvm-cli parsing is lenient elsewhere.)
- Override of any u8 is passed through verbatim (the interpreter number is advisory to the game; no validation needed).

## Testing

**zvm:**
- `default_interpreter_number`: `6 → 6`; `3/4/5/7/8 → 1`.
- After `init_caps` with `interpreter_number = None` on a v5 story: `mem[0x1E] == 1`.
- After `set_interpreter_number(Some(6))` + `init_caps` on a v5 story: `mem[0x1E] == 6` (and CP437 passthrough active — existing tests still pass).
- After `set_interpreter_number(Some(4))` + `init_caps`: `mem[0x1E] == 4`.

**app:**
- Config default deserializes `interpreter_number` as `None`.
- `GameSession::new(story, honor, Some(4))` yields `mem[0x1E] == 4`; `None` yields the version default (1 for v5). (Assert via an engine introspection or a header read available to the app tests.)

**zvm-cli:**
- `parse_interpreter(["-I","4","story.z5"]) == Some(4)` and the story path survives as a positional.
- `parse_interpreter(["--interpreter","3","story.z5"]) == Some(3)`.
- `parse_interpreter(["story.z5"]) == None`.

**Manual (post-implementation):** run BeyondZork under the new default and confirm the menu selection uses colour (not reverse) and the map box still renders; confirm `-I 6` restores the CP437/monochrome behaviour.

## Files Touched

- `crates/zvm/src/cpu/exec.rs` — `interpreter_number` field, `set_interpreter_number`, `default_interpreter_number`, `init_caps` threading; tests.
- `crates/zvm/src/screen.rs` — `init_header_caps` gains the `interpreter_number` parameter; `0x1E` write uses it.
- `crates/app/src/config.rs` — new `interpreter_number` field (mirror `honor_game_colours` at :376–377, its `#[serde(default)]` fn, the `Default` impl, the load-merge at :468, and the `toml_edit` write at :524).
- `crates/app/src/session.rs` — `GameSession::new` parameter + wiring.
- `crates/app/src/main.rs` (+ any other `GameSession::new` callers) — pass config value.
- `crates/zvm-cli/src/main.rs` — `-I` / `--interpreter` parsing + `build_machine` threading.

## Constraints

- `zvm` stays zero-dependency.
- Cross-platform.
- 0 warnings + full workspace suite green per task.
- `gvm` (Glulx) untouched.
