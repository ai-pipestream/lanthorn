# Gallery Options Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four Nerd Font Material Design arrow presets (with corner arrows), a distinct 4-icon portal preset, and a higher-fidelity gallery preview.

**Architecture:** New presets in `symbols.rs` (`Arrows`/`PortalGlyphs` `preset_names()`/`preset()`), guarded by a single-width validation test; an upgraded `render/gallery.rs` preview that renders a real box + all 8 arrows + a longer path + all 4 portal icons in the selected styles.

**Tech Stack:** Rust, ratatui 0.29, the existing symbol/gallery/style system.

## Global Constraints

- Glyph NAMES are authoritative; resolve any non-seeded codepoint from the Nerd Fonts `glyphnames.json` / MDI webfont CSS by name (do NOT invent codepoints). Verified seeds are in the spec.
- A wrong/wide codepoint must not break layout: a unit test asserts EVERY char in every new preset is single-width (reuse the existing width-validation path used by `apply_override`).
- Defaults unchanged: `filled` arrows + `ascii` portals stay the defaults; new families are opt-in.
- New preset names must be valid `[symbols]` `arrow_set`/`portal_icons` values (they persist via the existing gallery → style-file path).
- No `mapper`/`zvm` changes. Build + `cargo test --workspace` green and warning-clean after every task.
- Commit messages: NO backticks in the body; end every body with exactly:
  ```
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
  ```
- Spec: `docs/superpowers/specs/2026-06-24-gallery-options-design.md` (source of truth; read it — it has the verified codepoint table + the diagonal-fallback rule + the portal mapping).

## File structure
- **Modify `crates/app/src/symbols.rs`** — 4 new `Arrows` presets + 1 new `PortalGlyphs` preset; extend `preset_names()`/`preset()`.
- **Modify `crates/app/src/render/gallery.rs`** — preview upgrade.

---

### Task 1: Nerd Font arrow presets (bold/box/circle/outline) + corner arrows

**Files:** Modify `crates/app/src/symbols.rs`.

**Interfaces — Produces:** `Arrows::preset_names()` includes `"nf-bold","nf-box","nf-circle","nf-outline"` (after the existing `filled`/`line`/`nerdfont`); `Arrows::preset(name)` returns each. Cardinals from the spec's verified table; diagonals = the family's native MDI glyph if it exists in `glyphnames.json`, else the Unicode fallback `nw='↖' ne='↗' se='↘' sw='↙'`. (`nf-box` diagonals are verified: nw=F1968 ne=F196A sw=F1964 se=F1966.)

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn nf_arrow_presets_exist_and_are_single_width() {
    for name in ["nf-bold","nf-box","nf-circle","nf-outline"] {
        assert!(Arrows::preset_names().contains(&name), "{name} missing");
        let a = Arrows::preset(name).expect("preset");
        for ch in [a.north,a.south,a.east,a.west,a.ne,a.nw,a.se,a.sw] {
            assert!(!is_wide_estimate(ch), "{name}: wide char {:?}", ch);
        }
    }
    // verified cardinal codepoints for nf-bold:
    let b = Arrows::preset("nf-bold").unwrap();
    assert_eq!(b.north, '\u{F0737}');
    assert_eq!(b.south, '\u{F072E}');
    assert_eq!(b.east,  '\u{F0734}');
    assert_eq!(b.west,  '\u{F0731}');
    // nf-box native diagonals:
    let bx = Arrows::preset("nf-box").unwrap();
    assert_eq!(bx.ne, '\u{F196A}');
    assert_eq!(bx.nw, '\u{F1968}');
    assert_eq!(bx.se, '\u{F1966}');
    assert_eq!(bx.sw, '\u{F1964}');
}
```
(`is_wide_estimate` is the existing width check in symbols.rs — confirm its name/visibility; make `pub(crate)`/`pub` for the test if needed, or call the existing override-validation helper.)
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the four presets. Cardinals from the spec table. For diagonals: use `nf-box`'s verified F196x set; for `nf-bold`/`nf-circle`/`nf-outline` look up `arrow-{top-left,top-right,bottom-left,bottom-right}-<suffix>` in `glyphnames.json`/MDI — use the native glyph where present, else the Unicode diagonal. Document in the commit which families used native vs fallback diagonals.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(symbols): nf-md arrow presets (bold/box/circle/outline) + corner arrows".

---

### Task 2: Distinct 4-icon portal preset

**Files:** Modify `crates/app/src/symbols.rs`.

**Interfaces — Produces:** `PortalGlyphs::preset_names()` includes `"nerdfont-stairs"`; `preset("nerdfont-stairs")` returns up=`stairs-up`, down=`stairs-down`, in_=`location-enter`, out=`exit-run`, marker=`\u{F111}`, unknown=`\u{F059}`, path=`'┊'`, path_h=`'┄'`. Resolve the four direction codepoints by name from `glyphnames.json`/MDI (not seeded in the spec — pin and verify).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn nerdfont_stairs_portal_has_four_distinct_single_width_icons() {
    assert!(PortalGlyphs::preset_names().contains(&"nerdfont-stairs"));
    let p = PortalGlyphs::preset("nerdfont-stairs").unwrap();
    // four DISTINCT direction icons
    let four = [p.up, p.down, p.in_, p.out];
    for ch in four { assert!(!is_wide_estimate(ch)); }
    assert_eq!(four.iter().collect::<std::collections::HashSet<_>>().len(), 4, "up/down/in/out must differ");
}
```
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the preset; resolve `stairs-up`/`stairs-down`/`location-enter`/`exit-run` codepoints by name. Document the four codepoints used in the commit.
- [ ] **Step 4: Run, confirm pass; build clean.**
- [ ] **Step 5: Commit** — "feat(symbols): nerdfont-stairs portal preset (4 distinct icons)".

---

### Task 3: Gallery preview fidelity

**Files:** Modify `crates/app/src/render/gallery.rs`.

**Interfaces — Consumes:** the selected `SymbolSet` (box/arrows/portal/path). **Produces:** `draw_preview` renders, within the preview pane, a real room box (selected `BoxStyle`) with the 4 cardinal + 4 corner arrows, a longer multi-segment path (≥2 straights + ≥2 corners + ≥1 junction from the selected `PathGlyphs`), and all 4 portal icons (up/down/in/out). Degrades gracefully on small panes.

- [ ] **Step 1: Write the failing test** (TestBackend)
```rust
#[test]
fn preview_shows_box_corner_path_arrows_and_portals() {
    // build a GalleryState selecting box=thick, arrows=nf-box, portal=nerdfont-stairs, path=heavy;
    // render draw_preview into a TestBackend of adequate size;
    // assert the buffer contains: a thick box corner (┏), >=1 corner-arrow glyph,
    // >=2 distinct path glyphs, and the 4 portal glyphs (up/down/in/out chars present).
}
```
(Construct the scene from the current `draw_preview` signature — read it first; assert the selected glyphs appear in the rendered buffer.)
- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** the richer preview scene (box + 8 arrows at sides/corners + a path run with corners + a junction + the 4 portal icons), clamped to the pane; skip elements that don't fit on a tiny pane.
- [ ] **Step 4: Run full `cargo test --workspace`; green + warning-clean.**
- [ ] **Step 5: Commit** — "feat(gallery): higher-fidelity preview (box, 8 arrows, path, 4 portals)".

---

## Self-Review

**Spec coverage:**
- 4 nf-md arrow families + cardinals + corner arrows (native/fallback) → Task 1. ✅
- Distinct 4-icon portal preset (stairs/enter/exit) → Task 2. ✅
- Single-width validation (codepoint de-risk) → Tasks 1, 2. ✅
- Preview fidelity (box + 8 arrows + longer path + 4 portals) → Task 3. ✅
- Persist via existing style path / defaults unchanged → Global Constraints (no GalleryState change needed; selection indexes the larger preset_names lists). ✅
- No new categories / no renderer slot-logic change → Global Constraints + out-of-scope. ✅

**Placeholder scan:** Task 1/2 say "resolve by name from glyphnames.json" for the non-seeded codepoints — that's a concrete deterministic lookup against an authoritative file, with the single-width test as the safety net, not a vague directive. Task 3's TestBackend assertion is stated as concrete buffer-content checks; the implementer wires the current `draw_preview` scene.

**Type consistency:** `Arrows::preset_names/preset`, `PortalGlyphs::preset_names/preset`, the preset names (`nf-bold`,`nf-box`,`nf-circle`,`nf-outline`,`nerdfont-stairs`), `is_wide_estimate`, and the `Arrows`/`PortalGlyphs` field names match `symbols.rs`.

## Notes for the executor
- Tasks 1–2 are pure symbol-table additions (fully testable). Task 3 reads the current `draw_preview` + `GalleryState` scene before editing.
- The only research is resolving the non-seeded codepoints (bold/circle/outline diagonals; the four portal glyphs) by name — use the spec's verified seeds for the rest and the single-width test to catch mistakes.
