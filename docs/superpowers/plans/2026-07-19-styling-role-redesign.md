# Styling Role Redesign (SQ-0309) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 5 duplicated selector enumerations + 84-field `ColorScheme` with one
registry-driven role/derivation model, make the 11 Glk styles first-class, unify panel chrome,
and swap the interactive style UI for an auto-seeded commented template + `reload-style`.

**Architecture:** A single declarative **registry** (one row per selector: name, section, kind,
parent, delta, default) drives parsing, a **resolver** that computes a flat `theme: Map<Selector, Style/Glyph>`
via single-level parent fallback with per-slot **provenance**, template generation, and export.
Render sites read `theme.get(sel)` instead of `ColorScheme` fields.

**Tech Stack:** Rust; crates `app` (all styling), `gvm`/`zvm` (Glk style enum only — stay zero-dep).
Spec: `docs/design/2026-07-14-styling-role-redesign.md` (the source of truth for every value).

## Global Constraints
- The spec (`docs/design/2026-07-14-styling-role-redesign.md`) is authoritative for every selector,
  default, parent, delta, and the resolution chain. Each task's implementer MUST read the cited spec section.
- Pre-release: the `style.toml` schema breaks freely; NO back-compat decoder, NO old-file shim.
- `zvm`/`gvm` stay zero-dependency (std OK). All styling logic lives in `app`.
- Cross-platform (Windows/Linux/macOS): no unix-only calls in new code.
- Every themeable element is registry-declared (no hard-coded styles in render sites after migration).
- Glk host-snapshot format bumps once (glk slots gain modifiers); update `GLK_SNAPSHOT_VERSION` + tests.
- Keep the Z-machine render **byte-identical** for a game that touches no styles (Normal ≡ its base role).
- Verify external constants (Glk style numbers, stylehint hints) against gvm's `glk.rs` / an authoritative table.
- TDD: failing test → implement → green → commit. Frequent commits; stage files explicitly by path.

## Terminology (from spec)
**panels** = frames we draw (story/map/verb/debug/dialog/…). **windows** = story/VM Glk surfaces.
`panel.*` styles panels; `glk.*` styles windows.

---

## Wave 0 — Registry + resolver foundation (pure, no render changes)

### Task 0.1: Selector registry table
**Files:**
- Create: `crates/app/src/theme/registry.rs`
- Create: `crates/app/src/theme/mod.rs` (module root; `pub mod registry;`)
- Modify: `crates/app/src/lib.rs` (add `pub mod theme;`)
- Test: inline `#[cfg(test)]` in `registry.rs`

**Interfaces:**
- Produces: `pub enum Section { Roles, Elements, Panel, GlkBuffer, GlkGrid, Map, Debug, Statusbar }`
- Produces: `pub enum Kind { Style, BorderGlyphs, Placement }`
- Produces: `pub struct RegRow { pub name: &'static str, pub section: Section, pub kind: Kind, pub parent: Option<&'static str>, pub default_delta: Delta, }`
  where `Delta` holds optional fg/bg + modifier flags + optional glyph(s).
- Produces: `pub const REGISTRY: &[RegRow]` — one row for EVERY selector in the spec: 7 roles;
  the §2 elements; §2a/§2b `panel.*` (background, border, border:active, title, tab, tab:active,
  tab_divider, terminator_left, terminator_right); §3 `glk.buffer.*`/`glk.grid.*` (11 each, with
  canonical parent+delta); §4 `map.*` (colours + glyph-set presets); §4b `debug.*` (pc, tooltip,
  disasm_executed/rd/soft/data).

**Steps:**
- [ ] **Step 1: Write the failing test** — `registry_is_complete_and_unique`: assert every spec
  selector name is present (hard-code the expected name list from the spec), names are unique, each
  row's `parent` (if set) resolves to another registry name or a role, and every row has exactly one `Section`.
- [ ] **Step 2: Run it, watch it fail** — `cargo test -p app --lib theme::registry`.
- [ ] **Step 3: Implement `registry.rs`** — the enums, `RegRow`/`Delta`, and the `REGISTRY` table
  transcribing §2–§4b of the spec. No behavior wired yet.
- [ ] **Step 4: Green** — `cargo test -p app --lib theme::registry`.
- [ ] **Step 5: Commit** — `git add crates/app/src/theme/registry.rs crates/app/src/theme/mod.rs crates/app/src/lib.rs`

### Task 0.2: Resolver — flat theme map with parent fallback
**Files:**
- Create: `crates/app/src/theme/resolve.rs` (+ `pub mod resolve;` in mod.rs)
- Test: inline

**Interfaces:**
- Consumes: `REGISTRY`, a `Roles` value (7 resolved role Styles), and a `Decls` map (selector → partial override) — for now pass an empty `Decls`.
- Produces: `pub struct Theme { map: HashMap<String, Resolved> }` with `pub fn get(&self, sel: &str) -> Resolved`
  and `pub fn resolve(roles: &Roles, decls: &Decls) -> Theme`.
- Produces: `pub struct Resolved { pub style: ratatui::style::Style, pub glyph: Option<GlyphSet> }`.

**Steps:**
- [ ] **Step 1: Failing test** — `unset_selector_inherits_its_parent_role` (e.g. `transcript` == `text` role;
  `glk.buffer.header` == heading role + bold) and `explicit_decl_overrides_default`.
- [ ] **Step 2: Fail** — `cargo test -p app --lib theme::resolve`.
- [ ] **Step 3: Implement** — resolve each registry row: start from parent (role or another selector,
  single-level as the spec allows, resolved in dependency order), apply the row's default_delta, then any `Decls` override.
- [ ] **Step 4: Green.**
- [ ] **Step 5: Commit.**

### Task 0.3: Per-slot provenance
**Files:** Modify `crates/app/src/theme/resolve.rs`; Test: inline.

**Interfaces:**
- Produces: `pub enum Provenance { Default, GlobalUser, Garglk, PerGame }` on each `Resolved` (or a parallel map).
- Produces: `resolve` gains layered inputs: `resolve(roles, global: &Decls, garglk: &Decls, per_game: &Decls)`
  applying in the spec's static order (roles → defaults → global → garglk → per-game) and stamping provenance from the winning layer.

**Steps:**
- [ ] **Step 1: Failing test** — `per_game_layer_wins_and_is_stamped_pergame`; `unset_stays_default`.
- [ ] **Step 2–4: implement + green** (static build order per §Registry: per-game LAST).
- [ ] **Step 5: Commit.**

---

## Wave 1 — TOML schema + theme access facade

### Task 1.1: New TOML parser → `Decls`
**Files:**
- Create: `crates/app/src/theme/toml_schema.rs`
- Test: inline (+ a fixture string covering every section)

**Interfaces:**
- Consumes: raw `style.toml` text.
- Produces: `pub fn parse(text: &str) -> Result<ParsedStyle, Vec<String>>` where `ParsedStyle`
  carries `roles`, `decls` (elements/panel/glk/map/debug as selector→partial), `statusbar`,
  `transcript_rules`, and the `scheme` pointer. Sections per the spec's schema block.
- Value grammar reused from today (named/`palette:N`/`#hex`/256-index/`background`/`foreground`);
  factor the existing colour parser out of `style.rs` rather than duplicating.

**Steps:**
- [ ] **Step 1: Failing test** — parse the spec's full example (copy it to a test fixture): assert
  roles, a `[panel]` selector, a `[glk.buffer]` modifier, a `[map]` glyph preset, a `[debug]` tier
  glyph, a `[[transcript.rule]]`, and a `[statusbar.segment]` all round-trip into `ParsedStyle`.
- [ ] **Step 2: Fail.**
- [ ] **Step 3: Implement** the parser (toml crate already in use).
- [ ] **Step 4: Green.**
- [ ] **Step 5: Commit.**

### Task 1.2: `Theme` behind a `ColorScheme` facade
**Files:** Modify `crates/app/src/colors.rs`; Test: inline.

**Interfaces:**
- Produces: `ColorScheme` internally holds a `Theme` and exposes the existing field accessors as
  `fn transcript(&self) -> Style { self.theme.get("transcript").style }` etc. — a temporary facade so
  render sites keep compiling while Wave 4/5 migrate them to `theme.get`.
- Consumes: `parse` (Task 1.1) + `resolve` (Wave 0). `ColorScheme::resolve(cfg)` builds roles from the
  base scheme + `[roles]`, then the flat theme.

**Steps:**
- [ ] **Step 1: Failing test** — `facade_transcript_matches_resolved_theme`; the terminal-default theme
  through the registry equals today's hard-coded terminal defaults for a spot-check of ~10 selectors.
- [ ] **Step 2: Fail.**
- [ ] **Step 3: Implement** the facade + `ColorScheme::resolve` rebuild on the registry path.
- [ ] **Step 4: Green** + `cargo test -p app` (nothing else breaks via the facade).
- [ ] **Step 5: Commit.**

---

## Wave 2 — Glk slots gain modifiers + import (§3/§3a)

### Task 2.1: `GlkStyleColour` → full `Style` (+ snapshot bump)
**Files:** Modify `crates/app/src/colors.rs` (glk_styles), the Glk snapshot serializer, `crates/gvm/src/*` snapshot version; Tests: inline + snapshot round-trip.

**Interfaces:**
- Produces: glk slots are `[[Style; 11]; 2]` (fg/bg/mods), not `{fg,bg}`. Bump `GLK_SNAPSHOT_VERSION`.
- Consumes: registry glk defaults (Wave 0).

**Steps:**
- [ ] **Step 1: Failing test** — `glk_buffer_emphasized_default_is_italic`; snapshot round-trip preserves a modifier.
- [ ] **Step 2–4:** widen the type, bump the version, update the (dozen) `GlkStyleColour` initializers, green.
- [ ] **Step 5: Commit.**

### Task 2.2: ~~garglk.ini stylehint~~ + runtime stylehint → modifiers
**DROPPED / RE-SCOPED (2026-07-19).** The garglk.ini half targeted a directive that does not exist:
verified against Gargoyle, garglk.ini's `stylehint` is a global boolean (`0|1` → `honor_game_colours`,
already parsed); there is NO per-style `stylehint <wintype> <style> <hint> <value>` line (spec §3a
draft conflated the .ini directive with the Glk API). garglk.ini carries no per-style modifier signal
(bold/italic there are font files = terminal no-ops). So there is nothing to import from garglk.ini.

The REAL per-style modifier source is the game's runtime `glk_stylehint_set(wintype, style,
Weight/Oblique, value)` — gvm already records these (`weight`/`oblique` + resolver). Wiring them to
render modifiers is honor-gated and belongs with the runtime resolution chain: **folded into Wave 3
Task 3.1**. No standalone Wave-2 task. (Wave 2 = Task 2.1 only.)

---

## Wave 3 — Resolution chain + per-game precedence (§5)

### Task 3.1: Runtime chain with per-game lift
**Files:** Modify `crates/app/src/reload.rs` (build order), the per-cell resolve helpers (`render::resolve_glk_channel` and friends); Tests: inline.

**Interfaces:**
- Consumes: provenance (Task 0.3), `honor_game_colours`.
- Produces: static build applies per-game overlay LAST; runtime per-channel order
  `per-game(explicit) → garglk per-stream → game stylehint → slot → role → terminal`, with an
  explicit per-game slot acting as per-slot `honor_game_colours=off`. Global user theme still yields to the game.

**Steps:**
- [ ] **Step 1: Failing tests** — `explicit_per_game_slot_beats_game_stylehint`;
  `global_theme_still_yields_to_game_when_honor_on`; `shipped_garglk_ini_does_not_clobber_per_game`.
- [ ] **Step 2–4: implement + green.**
- [ ] **Step 5: Commit.**

### Task 3.2: Runtime game-stylehint MODIFIERS (folded in from old Task 2.2)
**Files:** the gvm→render bridge (whatever surfaces per-run/per-cell Glk `(wintype, style)` to the app),
`render::glk_theme_modifiers`/the buffer+grid render sites; Tests: inline. **Verify gvm already exposes
resolved Weight(4)/Oblique(5)** (`crates/gvm/src/glk.rs` `set_style_hint` + the ~1440 resolver) — read
it, do NOT bump the snapshot.

**Interfaces:**
- Consumes: gvm's recorded per-`(wintype, style)` Weight/Oblique; `honor_game_colours`.
- Produces: the game-stylehint layer contributes MODIFIERS too, not just colours — `weight 1`→BOLD,
  `oblique 1`→ITALIC (weight -1 / proportional / size = no-op), honor-gated exactly like the game
  colour channel, layered between the theme slot and the explicit per-game slot (§5 order).

**Steps:**
- [ ] **Step 1: Failing tests** — a game stylehint `Weight=1` on Emphasized renders BOLD when honor on,
  is ignored when honor off; `Oblique=1` → ITALIC. An explicit per-game slot still wins.
- [ ] **Step 2–4: implement + green** (constants verified against gvm's stylehint table).
- [ ] **Step 5: Commit.**

---

## Wave 4 — Panel unification (render) (§2a/§2b)

### Task 4.1: `panel.background` + `panel.border`/`:active` for all panels
**Files:** Modify `crates/app/src/render/paneframe.rs`, `main.rs` (focus→border pick), each panel renderer that draws its own frame; Tests: inline + render asserts.

**Interfaces:**
- Produces: a helper `panel_frame(focused: bool) -> (Style, BorderStyle, glyphs)` reading `panel.*`;
  every panel (story/map/verb/debug/dialog/hints/tidy/glyph-picker/room-info/file-browser) calls it.
- Consumes: `Theme` facade.

**Steps:**
- [ ] **Step 1: Failing test** — focused panel uses `panel.border:active` (bold), unfocused uses `panel.border`;
  panel body fill = `panel.background`.
- [ ] **Step 2–4: implement + green** (faithful to today's cyan/cyan+bold).
- [ ] **Step 5: Commit.**

### Task 4.2: Panel header chrome (title, tabs, divider, terminators)
**Files:** Modify the panel/tab renderers (`render/debug_panel.rs`, map layer-tab render, `render/paneframe.rs`); Tests: inline.

**Interfaces:**
- Produces: a shared header renderer using `panel.title`/`tab`/`tab:active`/`tab_divider`/`terminator_*`;
  terminator glyph defaults to the border style.
- Consumes: `Theme`.

**Steps:**
- [ ] **Step 1: Failing test** — debug window-tabs and map layer-tabs both style through `panel.tab*`;
  divider + terminator glyphs come from `panel.*` and terminator matches the border style.
- [ ] **Step 2–4: implement + green.**
- [ ] **Step 5: Commit.**

---

## Wave 5 — Domain migrations (§4/§4b)

### Task 5.1: `map.*` — merge symbols, portal_path_style, per-selector glyphs
**Files:** Modify `crates/app/src/render/map.rs`, symbols plumbing (`symbols.rs`), config (drop `[symbols]`); Tests: inline.

**Interfaces:**
- Produces: `map.*` reads colours + glyph-set presets from the theme; new `portal_path_style` for
  up/down/in/out connector lines (distinct from cardinal `path_style`); per-glyph overrides via a
  `glyphs` sub-map on `map.room`/`map.connector`. No `[symbols]` section, no glyph-override table.
- Consumes: `Theme`.

**Steps:**
- [ ] **Step 1: Failing tests** — `portal_path_style_renders_distinct_from_cardinal`;
  `map_room_glyph_override_applies`; symbol presets resolve from `[map]`.
- [ ] **Step 2–4: implement + green** (byte-check map render vs pre-migration for the default theme).
- [ ] **Step 5: Commit.**

### Task 5.2: `debug.*` — disasm-only + confidence tiers (SQ-0428)
**Files:** Modify `crates/app/src/render/debug_panel.rs`, disasm provenance in `crates/zvm/src/cpu/disasm_cache.rs` (expose per-unit tier), `Debugger` trait; Tests: inline.

**Interfaces:**
- Produces: `debug.*` holds only `pc`/`tooltip`/`disasm_{executed,rd,soft,data}`; each tier styles the
  line AND supplies its gutter `glyph` (executed default `|`, others blank). `exec_mark` removed.
  Debug panel body/frame/tabs come from `panel.*` (Wave 4).
- Consumes: `Theme`; a per-line confidence tier from the disasm cache.

**Steps:**
- [ ] **Step 1: Failing tests** — `executed_line_uses_disasm_executed_style_and_glyph`;
  `data_tier_is_muted_italic`; no `debug_exec_mark`/`debug_tab` references remain.
- [ ] **Step 2–4: implement + green** (this delivers SQ-0428). Close `Confirm: SQ-0428` if smoked.
- [ ] **Step 5: Commit.**

### Task 5.3: Render-site migration sweep
**Files:** All `crates/app/src/render/*.rs` + `main.rs` reading `ColorScheme` fields; Tests: existing suite.

**Interfaces:**
- Produces: every `state.colors.<field>` read becomes `theme.get("<selector>")`; the facade accessors
  (Task 1.2) are deleted at the end. This is the ~247-site sweep — split across as many commits as
  needed, one cohesive render module per commit, tests green after each.

**Steps:**
- [ ] **Step 1:** migrate one module (e.g. `render/transcript.rs`), run its tests.
- [ ] **Step 2:** repeat per module; keep `cargo test -p app` green after each.
- [ ] **Step 3:** delete the temporary facade accessors once no reads remain.
- [ ] **Step 4:** `cargo test -p app` + `cargo clippy -p app` clean.
- [ ] **Step 5: Commit** (per module).

---

## Wave 6 — Remove interactive UI + template delivery

### Task 6.1: Remove style editor + glyph picker + symbol gallery
**Files:** Delete `render/style_editor.rs`, `style_actions.rs`, `style_mru.rs`, `render/glyph_picker.rs`,
`glyph_actions.rs`, `render/gallery.rs`; prune `slash.rs` (`open-style-editor`, `open-gallery`),
`input.rs` (`OpenGallery`, all `GlyphPicker*`, `StyleOpenGlyphPicker`, gallery sub-mode), `keymap.rs`
(`f`→open-gallery binding), `state.rs`/`overlays.rs` (their overlay state), and their tests.
KEEP `cover_gallery.rs` (SQ-0374). Tests: build + existing suite.

**Steps:**
- [ ] **Step 1:** delete the files + remove every reference (compiler drives completeness).
- [ ] **Step 2:** `cargo build -p app` clean (no dangling refs); remove now-orphaned imports.
- [ ] **Step 3:** `cargo test -p app` green; update/remove tests that referenced the removed UI.
- [ ] **Step 4: Commit.**

### Task 6.2: Registry-driven commented template + auto-seed + regen example
**Files:** Create `crates/app/src/theme/template.rs`; modify `styles.rs` (writers), `startup.rs`
(auto-seed if missing), regenerate repo `style.example.toml`; wire `reload-style` through the new schema. Tests: inline.

**Interfaces:**
- Produces: `pub fn commented_template() -> String` — every registry selector, grouped by section,
  fully commented, with a per-section explanatory comment (roles, panels, glk.*, map, debug, transcript
  rules, statusbar); startup writes it to the user's `style.toml` ONLY if absent (never overwrites).
- Consumes: `REGISTRY`.

**Steps:**
- [ ] **Step 1: Failing tests** — `template_covers_every_registry_selector`;
  `template_parses_clean_and_resolves_to_defaults` (uncommenting nothing == registry defaults);
  `auto_seed_writes_when_missing_and_never_overwrites`.
- [ ] **Step 2–4: implement + green;** regenerate `style.example.toml` from `commented_template()` and
  assert the repo file matches (the existing `style_example_toml_parses` test updates to the new schema).
- [ ] **Step 5: Commit.**

---

## Wave 7 — Teardown + finalize

### Task 7.1: Delete the 5 duplicated enumerations + 84-field ColorScheme
**Files:** `crates/app/src/style.rs` (`SELECTOR_FIELDS`, `SELECTOR_GROUPS`, `style_for_selector`,
`apply_color_decls`, `write_style_full` old path), `colors.rs` (the 84 fields, now behind the theme). Tests: full suite.

**Steps:**
- [ ] **Step 1:** remove each enumeration once the registry path fully replaces it; compiler + tests drive it.
- [ ] **Step 2:** `cargo test -p app` + `cargo test -p zvm` + `cargo test -p gvm` green; `cargo clippy` clean; VMs still zero-dep.
- [ ] **Step 3: Commit.**

### Task 7.2: Docs + smoke checklist
**Files:** README (styling section — new schema, no editor, reload-style, template), `style.example.toml`. Tests: docs only.

**Steps:**
- [ ] **Step 1:** update README styling docs to the new model; note the format break.
- [ ] **Step 2:** record the manual smoke list (needs TTY): default theme visually stable on a Z-machine
  title + CM + Kerkerkruip; per-game override beats a game stylehint; auto-seeded template appears + reload works;
  panel focus border; debug confidence colours. Add to the to-verify memory; SQ-0309 → `confirm`.
- [ ] **Step 3: Commit.**

---

## Self-Review notes
- Spec coverage: Waves 0–3 build the model (registry/resolve/schema/glk-modifiers/precedence); Waves 4–5
  migrate render (panels, map, debug, all sites); Waves 6–7 remove the old UI + enumerations + ship the template.
- Byte-identity guard (no-style game, default-theme map render) appears in Tasks 1.2 and 5.1.
- Provenance (0.3) is the prerequisite for the per-game lift (3.1) — keep that order.
- Each wave leaves the tree green; the facade (1.2) lets Waves 2–4 land before the 5.3 sweep.
