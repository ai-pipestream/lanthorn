# Configurable Map Symbols Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Centralize every map glyph into one configurable `SymbolSet`, resolved from `[symbols]` config (named presets + per-glyph overrides), with defaults that reproduce today's rendering exactly.

**Architecture:** New app-side `crates/app/src/symbols.rs` owns `SymbolSet` (all glyphs) and presets. `Config` (Track B) gains a `symbols` section; `SymbolSet::resolve(&Config)` builds the set. `AppState` carries the resolved set; `render/map.rs` reads `state.symbols.*` instead of literals. `mapper` is untouched.

**Tech Stack:** Rust workspace, ratatui 0.29, serde + toml (already in `crates/app`), `mapper::graph`/`render`.

## Global Constraints

- Defaults MUST reproduce today's glyphs byte-for-byte: a frame rendered with `SymbolSet::default()` is identical to current output.
- `mapper` crate is NOT modified — glyphs are app-side only.
- Colors are OUT OF SCOPE — do not touch `CURRENT_STYLE`/`SELECTED_STYLE`/`NORMAL_STYLE`/`CONNECTOR_STYLE` or any `Color`. Selection stays visible via the existing yellow color.
- Outline flavor precedence: `current > portal > selected > normal`.
- Override values must be single-display-width; invalid (empty/multi-char/wide) → keep the preset glyph.
- TDD, YAGNI, surgical. Match existing style. Commit messages: no backticks; end with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

### Confirmed current glyphs (the defaults)

- Room outline tuple order is `(tl, tr, bl, br, h, v)`:
  - normal (rounded): `╭ ╮ ╰ ╯ ─ │`  (map.rs:1135 / 1228)
  - current (heavy): `┏ ┓ ┗ ┛ ━ ┃`  (map.rs:1131 / 1224)
  - portal (double): `╔ ╗ ╚ ╝ ═ ║`  (map.rs:1133 / 1226)
  - selected: defaults to the **normal** set (today selection is color-only)
- Cardinal arrows (`arrow_for_departure`, map.rs:214): N `▲`, S `▼`, E `▶`, W `◀`
- Diagonal arrows (`DIAG_*`, map.rs:180): NE `↗`, NW `↖`, SE `↘`, SW `↙`
- Path line-art (`glyph_for`, map.rs:478): EW `─`, NS `│`, SE `┌`, SW `┐`, NE `└`, NW `┘`, NSE `├`, NSW `┤`, EWS `┬`, EWN `┴`, NESW `┼`
- Portal marker (`draw_portal_icons`, map.rs:~1079): `●`; portal connector style is `Color::Cyan` (`draw_portal_connectors`, map.rs:~889). Exact portal-icon/path glyph slots are confirmed in Task 7's investigation.

---

### Task 1: `SymbolSet` data model with back-compat defaults

**Files:**
- Create: `crates/app/src/symbols.rs`
- Modify: `crates/app/src/lib.rs` (add `pub mod symbols;`)

**Interfaces:**
- Produces: `pub struct BoxStyle { pub tl: char, pub tr: char, pub bl: char, pub br: char, pub h: char, pub v: char }`; `pub struct Arrows { pub north: char, pub south: char, pub east: char, pub west: char, pub ne: char, pub nw: char, pub se: char, pub sw: char }`; `pub struct PathGlyphs { pub ew, ns, se, sw, ne, nw, nse, nsw, ews, ewn, nesw: char }`; `pub struct PortalGlyphs { pub marker: char, pub path: char }`; `pub struct SymbolSet { pub room_normal: BoxStyle, pub room_current: BoxStyle, pub room_portal: BoxStyle, pub room_selected: BoxStyle, pub arrows: Arrows, pub path: PathGlyphs, pub portal: PortalGlyphs }`; `impl Default for SymbolSet`.

- [ ] **Step 1: Write the failing test** in `symbols.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_matches_todays_glyphs() {
        let s = SymbolSet::default();
        assert_eq!((s.room_normal.tl, s.room_normal.br, s.room_normal.h), ('╭', '╯', '─'));
        assert_eq!((s.room_current.tl, s.room_current.v), ('┏', '┃'));
        assert_eq!((s.room_portal.tl, s.room_portal.v), ('╔', '║'));
        // selected defaults to the normal set (color-only selection today)
        assert_eq!((s.room_selected.tl, s.room_selected.v), (s.room_normal.tl, s.room_normal.v));
        assert_eq!((s.arrows.north, s.arrows.east, s.arrows.ne), ('▲', '▶', '↗'));
        assert_eq!((s.path.ew, s.path.nesw, s.path.se), ('─', '┼', '┌'));
        assert_eq!(s.portal.marker, '●');
    }
}
```

- [ ] **Step 2: Run to verify it fails:** `cargo test -p app symbols::tests::default_matches` — FAIL (module/types not defined).
- [ ] **Step 3: Implement** the structs above and `impl Default for SymbolSet` filling every field from the "Confirmed current glyphs" list (selected = clone of normal). Add `pub mod symbols;` to `lib.rs` (alphabetical with the other `pub mod` lines).
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app symbols::` — PASS.
- [ ] **Step 5: Commit:** `git add crates/app/src/symbols.rs crates/app/src/lib.rs && git commit` — "feat(symbols): SymbolSet data model with today's glyphs as defaults".

---

### Task 2: Per-category presets

**Files:**
- Modify: `crates/app/src/symbols.rs`

**Interfaces:**
- Produces: `impl BoxStyle { pub fn preset(name: &str) -> Option<BoxStyle> }` (names: `rounded`, `thick`, `double`, `ascii`, `borderless`); `impl Arrows { pub fn preset(name: &str) -> Option<Arrows> }` (`filled`, `line`, `nerdfont`); `impl PathGlyphs { pub fn preset(name: &str) -> Option<PathGlyphs> }` (`light`, `heavy`, `dotted`); `impl PortalGlyphs { pub fn preset(name: &str) -> Option<PortalGlyphs> }` (`ascii`, `nerdfont`). Unknown name → `None`.

- [ ] **Step 1: Write the failing test:**

```rust
#[test]
fn presets_resolve_and_default_names_match_default_set() {
    assert_eq!(BoxStyle::preset("rounded"), Some(SymbolSet::default().room_normal));
    let ascii = BoxStyle::preset("ascii").unwrap();
    assert_eq!((ascii.tl, ascii.h, ascii.v), ('+', '-', '|'));
    let borderless = BoxStyle::preset("borderless").unwrap();
    assert_eq!(borderless.h, ' ');
    assert_eq!(Arrows::preset("filled"), Some(SymbolSet::default().arrows));
    assert_eq!(PathGlyphs::preset("light"), Some(SymbolSet::default().path));
    assert!(BoxStyle::preset("nonsense").is_none());
}
```

(Requires `#[derive(PartialEq, Eq, Clone, Copy, Debug)]` on the glyph structs — add it.)

- [ ] **Step 2: Run to verify it fails:** `cargo test -p app presets_resolve` — FAIL.
- [ ] **Step 3: Implement** each `preset`. `rounded`/`filled`/`light`/`ascii`(portal) return the same values as `Default`. Define the other presets: `thick` = the heavy tuple, `double` = the double tuple, `ascii` = `+ + + + - |`, `borderless` = all spaces; `line` arrows = `↑↓→←↗↖↘↙`, `nerdfont` arrows/portal = single-width nerdfont codepoints (document chosen codepoints in comments); `heavy` path = heavy box-drawing line set, `dotted` path = `┄┆` style dotted set. Keep each preset's slots single-width.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app symbols::` — PASS.
- [ ] **Step 5: Commit:** "feat(symbols): named presets per glyph category".

---

### Task 3: `SymbolConfig` + `SymbolSet::resolve` (presets + overrides + width validation)

**Files:**
- Modify: `crates/app/src/config.rs` (add `SymbolConfig`, add `symbols` field to `Config`)
- Modify: `crates/app/src/symbols.rs` (add `resolve`)

**Interfaces:**
- Consumes: `crate::config::Config`.
- Produces: in `config.rs`: `#[derive(Debug, Deserialize)] pub struct SymbolConfig { #[serde(default = "..")] pub box_style: String, .. arrow_set, portal_icons, path_style: String, #[serde(default)] pub overrides: std::collections::BTreeMap<String, String> }` with `Default`; `Config` gains `#[serde(default)] pub symbols: SymbolConfig`. In `symbols.rs`: `impl SymbolSet { pub fn resolve(cfg: &crate::config::SymbolConfig) -> SymbolSet }`.

- [ ] **Step 1: Write failing tests** in `symbols.rs`:

```rust
#[test]
fn resolve_default_config_equals_default_set() {
    let cfg = crate::config::SymbolConfig::default();
    assert_eq!(SymbolSet::resolve(&cfg), SymbolSet::default());
}
#[test]
fn resolve_applies_preset_then_override() {
    let mut cfg = crate::config::SymbolConfig::default();
    cfg.box_style = "ascii".into();
    cfg.overrides.insert("room.normal.tl".into(), "#".into());
    let s = SymbolSet::resolve(&cfg);
    assert_eq!(s.room_normal.tl, '#');           // override beats preset
    assert_eq!(s.room_normal.h, '-');            // rest from ascii preset
}
#[test]
fn resolve_rejects_bad_width_override() {
    let mut cfg = crate::config::SymbolConfig::default();
    cfg.overrides.insert("arrow.north".into(), "ab".into());   // multi-char
    cfg.overrides.insert("arrow.south".into(), "".into());     // empty
    let s = SymbolSet::resolve(&cfg);
    assert_eq!(s.arrows.north, SymbolSet::default().arrows.north); // unchanged
    assert_eq!(s.arrows.south, SymbolSet::default().arrows.south);
}
```

(`SymbolSet` needs `#[derive(PartialEq, Debug)]`.)

- [ ] **Step 2: Run to verify it fails:** `cargo test -p app resolve_` — FAIL.
- [ ] **Step 3: Implement.** Add `SymbolConfig` (each preset-name field defaults via a `fn default_box_style() -> String { "rounded".into() }` etc., matching the default preset names) and the `Config.symbols` field. Implement `resolve`: start from each category preset (falling back to `Default` when the name is unknown), then for each `(key, val)` in `overrides`: parse `val` as exactly one `char` whose display width is 1 (use `val.chars().count() == 1` plus a width check — `unicode-width` is NOT a dependency; approximate "single-width" by rejecting any char in a known-wide range OR simply require `val.chars().count() == 1` and treat as width 1; document the chosen rule), map the dotted `key` to the slot via a `match` over the slot map (see spec), and assign. Unknown keys and invalid values are ignored.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app symbols::` and `cargo test -p app config::` — PASS.
- [ ] **Step 5: Commit:** "feat(symbols): resolve SymbolSet from config presets + overrides".

---

### Task 4: Carry `SymbolSet` in `AppState` and resolve at startup

**Files:**
- Modify: `crates/app/src/state.rs` (add field)
- Modify: `crates/app/src/main.rs` (resolve from `cfg` after `state` is created — near `crates/app/src/main.rs:260`)

**Interfaces:**
- Consumes: `SymbolSet::resolve`, `cfg: Config` (already built at `main.rs` startup by Track B).
- Produces: `AppState.symbols: crate::symbols::SymbolSet`.

- [ ] **Step 1: Write the failing test** in `state.rs`:

```rust
#[test]
fn appstate_default_symbols_are_default_set() {
    let st = AppState::default();
    assert_eq!(st.symbols, crate::symbols::SymbolSet::default());
}
```

- [ ] **Step 2: Run to verify it fails:** `cargo test -p app appstate_default_symbols` — FAIL.
- [ ] **Step 3: Implement.** Add `pub symbols: crate::symbols::SymbolSet` to `AppState` (its `Default` derive fills it via `SymbolSet::default()`; if `AppState` is hand-implemented Default, set it there). In `main.rs`, right after `let mut state = AppState::default();` add `state.symbols = app::symbols::SymbolSet::resolve(&cfg.symbols);`.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app` and `cargo build -p app` — PASS.
- [ ] **Step 5: Commit:** "feat(symbols): carry resolved SymbolSet in AppState".

---

### Task 5: Renderer reads outlines from `state.symbols` (+ precedence)

**Files:**
- Modify: `crates/app/src/render/map.rs` (`draw_compact_room` ~1118, `draw_box_room` ~1208, and their call sites in `render_map`)

**Interfaces:**
- Consumes: `AppState.symbols`. The two draw fns currently receive `style: Style` and derive `is_current` from `Modifier::REVERSED` (map.rs:1128). Pass them `&SymbolSet`, `room.has_layer_portal`, and a `selected: bool` (computed at the call site as `state.selected_room == Some(room.id)`). Define `fn outline_for(sym: &SymbolSet, is_current: bool, has_portal: bool, selected: bool) -> &BoxStyle` applying precedence: current → `room_current`; else has_portal → `room_portal`; else selected → `room_selected`; else `room_normal`.

- [ ] **Step 1: Write the failing back-compat + preset test** (TestBackend) in `render/map.rs` tests. Model on the existing `renders_current_room_highlighted_into_buffer` test (map.rs:1953). Render a small map with `state.symbols = SymbolSet::default()` and assert a normal room's corner cell is `╭`; then set `state.symbols = SymbolSet::resolve(&{box_style:"ascii"})` and assert the same cell is `+`.
- [ ] **Step 2: Run to verify it fails:** `cargo test -p app <new test>` — FAIL (still reads literals).
- [ ] **Step 3: Implement.** Replace the `if is_current {..} else if room.has_layer_portal {..} else {..}` literal tuples in BOTH `draw_compact_room` and `draw_box_room` with a lookup via `outline_for(&state.symbols, is_current, room.has_layer_portal, selected)` returning the `BoxStyle`; thread `&SymbolSet` and `selected: bool` through the call sites in `render_map`. Do not touch any `Style`/color.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app render::map` — PASS, including the pre-existing render tests (back-compat).
- [ ] **Step 5: Commit:** "feat(symbols): render room outlines from SymbolSet".

---

### Task 6: Renderer reads arrows from `state.symbols`

**Files:**
- Modify: `crates/app/src/render/map.rs` (`diagonal_arrow` ~186, `arrow_for_departure` ~214, callers)

**Interfaces:**
- Consumes: `state.symbols.arrows`. Change `diagonal_arrow`/`arrow_for_departure` to take `&Arrows` and return `char` (or have callers index `arrows` directly). Update callers in `render_lane_connectors`/`draw_connector_arrows`/`draw_portal_connectors`.

- [ ] **Step 1: Write the failing test:** TestBackend render asserting a due-east connector arrowhead cell is `▶` with defaults, and changes to `→` with `arrow_set = "line"`.
- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement** the parameter change; map `Side::Right→arrows.east`, `Left→west`, `Top→north`, `Bottom→south`; `Direction::NE→arrows.ne` etc.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app render::map` — PASS.
- [ ] **Step 5: Commit:** "feat(symbols): render arrows from SymbolSet".

---

### Task 7: Renderer reads path line-art + portal glyphs from `state.symbols`

**Files:**
- Modify: `crates/app/src/render/map.rs` (`glyph_for` ~478, `draw_portal_icons` ~1006, `draw_portal_connectors` ~881)

**Interfaces:**
- Consumes: `state.symbols.path`, `state.symbols.portal`. `glyph_for(mask)` becomes `glyph_for(mask, path: &PathGlyphs) -> Option<char>`.

- [ ] **Step 0 (investigate):** Read `draw_portal_icons` (map.rs:1006-1090) and `draw_portal_connectors` (map.rs:881-905) fully. Confirm which glyph literals are the portal MARKER (`●`) and the portal PATH connector char. If portal up/down BADGE arrows (`↑`/`↓`) are produced in `mapper` or `draw_stub` rather than here, they are OUT OF SCOPE for the app-side SymbolSet — note that in a comment and do not change them.
- [ ] **Step 1: Write the failing test:** TestBackend render asserting (a) a junction connector cell is `┼`/`├` etc. with defaults and changes under `path_style = "heavy"`; (b) the portal marker cell is `●` with defaults. Build the smallest map that exercises a multi-direction junction and a portal room.
- [ ] **Step 2: Run to verify it fails** — FAIL.
- [ ] **Step 3: Implement.** Replace the `glyph_for` literal table with `path` fields (map each mask to the matching `PathGlyphs` slot); replace the `●` marker with `portal.marker` and the portal connector char with `portal.path`. Leave any `Color` alone.
- [ ] **Step 4: Run to verify it passes:** `cargo test -p app` (full app suite) — PASS.
- [ ] **Step 5: Commit:** "feat(symbols): render path line-art and portal glyphs from SymbolSet".

---

## Notes for the implementer

- After Task 7, grep `crates/app/src/render/map.rs` for any remaining box-drawing/arrow literals to confirm every family is centralized (except the documented out-of-scope up/down badges, if any).
- The existing render tests (e.g. `renders_current_room_highlighted_into_buffer`, the `╔`/`║` assertion at map.rs:3416) are the back-compat guard — they must keep passing unchanged at every task.
- Do not add `unicode-width`; the single-width override rule is `chars().count() == 1` plus rejecting the known wide-CJK/emoji ranges (document the exact rule in `resolve`).
