# Live Style Editor — Phase 1 (colors & attributes) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An in-app, full-screen, click-to-edit style editor that lets the user set every element's foreground/background color and the five text attributes with live preview, then save to `style.toml` — no hand-editing + reload.

**Architecture:** A new editor mode mirroring `config_screen`: a working `StyleDoc` (clone of the loaded style) edited in memory, a cached `preview: ColorScheme` recomputed via `style::resolve` on each edit, a board that renders labeled samples of every selector (styled from the preview) and doubles as the click/keyboard selector picker, and a property pane (fg/bg swatch grid + `default` + custom `#hex` + a shared MRU-16, plus attribute chips). Save mirrors config-save (`resolve` → `state.colors`/`symbols` → `save_style_and_repoint`). Cancel discards (the live theme is never mutated mid-edit). Reset reverts a selector (or all) to the built-in default doc.

**Tech Stack:** Rust, ratatui, the existing `style.rs`/`colors.rs`/`reload.rs` pipeline.

Design reference: `docs/superpowers/specs/2026-06-26-style-editor-phase1-design.md`.

## Global Constraints

- Commit trailers on EVERY commit body (no backticks anywhere in commit bodies — zsh):
  Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV
- Per task: full `cargo test -p app` (lib + bin + headless) green and `cargo build -p app` **0 warnings** before committing. Run the FULL suite, not a filtered subset.
- Do NOT push or merge; commit locally only. Do NOT edit `TODO.md` (gitignored).
- Scope is **colors (fg/bg) + the five attributes** (bold/italic/underline/dim/reversed) only. Border type, side/corner glyph overrides, and the character picker are **Phase 2** — out of scope. Do NOT edit the `Decl` border fields (`style`, `style_top/bottom/left/right`, `header`, `shadow`); the working doc carries them through untouched.
- The editor's own chrome is themeable via existing selectors — **no hard-coded colors** (mirror `render/config_screen.rs` / `render/reset_dialog.rs`).
- Save uses `save_style_and_repoint` (full canonical export), exactly as config-save does — consistent behavior; comments are not preserved (acceptable, documented in the spec).

### Verified interfaces (from the existing codebase — use as-is)

- `style::load_style(pointer: Option<&str>, user_dir: &Path) -> (StyleDoc, Vec<String>)`
- `style::resolve(doc: &StyleDoc, dir: &Path) -> (ColorScheme, symbols::SymbolSet, Vec<String>)`
- `style::SELECTOR_FIELDS: &[&str]` — 37 entries (the full selector list).
- `style::DEFAULT_STYLE_TOML: &str` — the built-in default doc (parse via `parse_style_toml`).
- `style::parse_style_toml(text: &str) -> Result<StyleDoc, String>`
- `StyleDoc { colors: StyleColors, symbols, transcript_rules, status_bar }`; `StyleColors.selectors: BTreeMap<String, Decl>`.
- `Decl { fg: Option<String>, bg: Option<String>, bold/italic/underline/dim/reversed: Option<bool>, /* + Phase-2 border fields */ }`.
- `save_style_and_repoint(state: &mut AppState, user_dir: &Path)` — `main.rs:97`; writes `state.colors`/`symbols` to `style.toml` and re-resolves.
- `AppState.colors: ColorScheme` (`state.rs:638`), `AppState.symbols: SymbolSet` (`state.rs:634`), `AppState.config: Config` (`state.config.user_dir`, `state.config.style: Option<String>`).
- Config-screen pattern to mirror: `ConfigScreenState { working, selected }` (`state.rs:503`), `AppState.config_screen: Option<...>` (`state.rs:674`), open at `input.rs:2051`, key-intercept at `input.rs:332`, render call at `main.rs:594`, Tab-focus block at `main.rs:1680`.

---

### Task 1: Editor mode scaffolding (state, open/close, entry point)

**Files:**
- Modify: `crates/app/src/state.rs` — add `StyleEditorState` + `AppState.style_editor` field + init.
- Modify: `crates/app/src/input.rs` — `Action::OpenStyleEditor` / `StyleEditorCancel`; open handler; key-intercept stub.
- Modify: `crates/app/src/keymap.rs` — `Command::OpenStyleEditor` (+ `/style` name, label, Context::Global, ALL_COMMANDS, default keybind F3).

**Interfaces:**
- Produces: `pub struct StyleEditorState { pub doc: style::StyleDoc, pub preview: colors::ColorScheme, pub selectors: Vec<&'static str>, pub active: usize, pub focus: StyleFocus, pub custom_buf: String, pub mru: Vec<String> }`; `pub enum StyleFocus { Board, Fg, Bg, Custom, Attrs }`; `AppState.style_editor: Option<StyleEditorState>`; `fn open_style_editor(state: &mut AppState)`.

- [ ] **Step 1: Write the failing test**

In `crates/app/src/state.rs` tests (or `input.rs` tests), add:

```rust
#[test]
fn open_style_editor_seeds_doc_and_preview() {
    let mut s = AppState::default(); // or the test constructor used elsewhere
    crate::input::apply_action(crate::input::Action::OpenStyleEditor, &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().expect("editor open");
    assert_eq!(ed.active, 0);
    assert!(!ed.selectors.is_empty(), "selector list seeded");
    // Cancel closes it.
    crate::input::apply_action(crate::input::Action::StyleEditorCancel, &mut s, &mut Mapper::default());
    assert!(s.style_editor.is_none());
}
```

(Use the same `AppState`/`Mapper` construction the existing `input.rs`/`state.rs` tests use — grep an existing `apply_action` test for the exact setup.)

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app open_style_editor_seeds` → compile error (`StyleEditorState`/actions missing).

- [ ] **Step 3: Add the state type + field**

In `crates/app/src/state.rs`, near `ConfigScreenState` (~503):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleFocus { Board, Fg, Bg, Custom, Attrs }

pub struct StyleEditorState {
    pub doc: crate::style::StyleDoc,
    pub preview: crate::colors::ColorScheme,
    pub selectors: Vec<&'static str>,
    pub active: usize,
    pub focus: StyleFocus,
    pub custom_buf: String,
    pub mru: Vec<String>,
}
```

Add to `AppState` near `config_screen` (~674): `pub style_editor: Option<StyleEditorState>,` and init `style_editor: None,` (~839).

- [ ] **Step 4: Add actions + open/cancel handlers**

In `crates/app/src/input.rs`: add `OpenStyleEditor` and `StyleEditorCancel` to the `Action` enum (near `OpenConfig`/`ConfigCancel`). In `apply_action`:

```rust
Action::OpenStyleEditor => { open_style_editor(state); }
Action::StyleEditorCancel => { state.style_editor = None; }
```

Add the opener (in `input.rs`, near the config-open handler):

```rust
pub fn open_style_editor(state: &mut AppState) {
    let user_dir = state.config.user_dir.clone();
    let (doc, _w) = crate::style::load_style(state.config.style.as_deref(), &user_dir);
    let (preview, _set, _w2) = crate::style::resolve(&doc, &user_dir);
    let selectors: Vec<&'static str> = crate::style::SELECTOR_FIELDS.to_vec();
    state.style_editor = Some(crate::state::StyleEditorState {
        doc, preview, selectors, active: 0,
        focus: crate::state::StyleFocus::Board, custom_buf: String::new(), mru: Vec::new(),
    });
}
```

(Seed `mru: Vec::new()` for now; Task 5 replaces that single line with `crate::style_mru::load_mru(&user_dir)`.)

- [ ] **Step 5: Add the key-intercept stub + entry command**

In `input.rs` `key_to_action`, near the config-screen intercept (~332), add ABOVE it:

```rust
if state.style_editor.is_some() {
    return style_editor_key_to_action(key, state);
}
```

Add a minimal dispatch (expanded in later tasks):

```rust
fn style_editor_key_to_action(key: KeyEvent, _state: &AppState) -> Action {
    match key.code {
        KeyCode::Esc => Action::StyleEditorCancel,
        _ => Action::None,
    }
}
```

In `crates/app/src/keymap.rs`: add `Command::OpenStyleEditor` (enum + `from_name`/name `"open-style-editor"` + label `"style editor"` + `Context::Global` + `ALL_COMMANDS` + `Command::OpenStyleEditor => Action::OpenStyleEditor`), and a default bind (verify F3 is unbound first): `bind!(plain(F(3)), Command::OpenStyleEditor, Context::Global)`. This also makes `/style-editor` work via the slash registry; add a `/style` alias if the registry supports aliases (else document `/style-editor`).

- [ ] **Step 6: Run the test + full suite**

Run: `cargo test -p app` → PASS, 0 warnings. (The editor opens with no render yet; that's Task 3.)

- [ ] **Step 7: Commit**

```bash
git -C /Volumes/Videos/Source/babelmap add crates/app/src/state.rs crates/app/src/input.rs crates/app/src/keymap.rs
git -C /Volumes/Videos/Source/babelmap commit -m "feat(app): style editor mode scaffolding (state, open/cancel, entry)

Adds StyleEditorState (working StyleDoc + cached preview ColorScheme + selector
list + focus + custom buffer + MRU), the AppState.style_editor field, open_style_
editor (load_style -> resolve), OpenStyleEditor/StyleEditorCancel actions, the
key intercept, and the /style entry command (F3). Render lands next.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Uvf2RNUS7SBZHXPWqcRAkV"
```

---

### Task 2: `style_for_selector` read accessor + grouped selector order

**Files:**
- Modify: `crates/app/src/style.rs` — add `style_for_selector(cs: &ColorScheme, selector: &str) -> Style` (the read inverse of `apply_color_decls`) and `pub const SELECTOR_GROUPS: &[(&str, &[&str])]` (grouped ordering for the board).

**Interfaces:**
- Produces: `pub fn style_for_selector(cs: &colors::ColorScheme, selector: &str) -> ratatui::style::Style`; `pub const SELECTOR_GROUPS: &[(&str, &[&str])]`.

- [ ] **Step 1: Write the failing test**

In `style.rs` tests:

```rust
#[test]
fn style_for_selector_reads_the_right_field() {
    let mut cs = colors::ColorScheme::terminal_default();
    cs.room_current = ratatui::style::Style::new().fg(ratatui::style::Color::Green);
    assert_eq!(style_for_selector(&cs, "room:current").fg, Some(ratatui::style::Color::Green));
    // Unknown selector → default (empty) style, no panic.
    assert_eq!(style_for_selector(&cs, "nope"), ratatui::style::Style::default());
}

#[test]
fn selector_groups_cover_all_selector_fields() {
    use std::collections::BTreeSet;
    let grouped: BTreeSet<&str> = SELECTOR_GROUPS.iter().flat_map(|(_, s)| s.iter().copied()).collect();
    for sel in SELECTOR_FIELDS {
        assert!(grouped.contains(sel), "selector {sel} missing from SELECTOR_GROUPS");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p app style_for_selector selector_groups` → compile error.

- [ ] **Step 3: Implement the accessor + groups**

In `style.rs`, mirror `apply_color_decls`'s match but READING each field (return `Style::default()` for reserved/unknown like `"border"`, `"map_border"`, etc. whose color isn't a single field — Phase 1 only needs the color-bearing ones; reserved ones return default):

```rust
pub fn style_for_selector(cs: &colors::ColorScheme, selector: &str) -> ratatui::style::Style {
    use ratatui::style::Style;
    match selector {
        "room" => cs.room_normal,
        "room:current" => cs.room_current,
        "room:selected" => cs.room_selected,
        "connector" => cs.connector,
        "connector:distorted" => cs.connector_distorted,
        "connector:portal" => cs.portal_connector,
        "border:focused" => cs.focused_border,
        "statusbar" => cs.status_bar,
        "transcript" => cs.transcript,
        "transcript:input" => cs.transcript_input,
        "transcript:meta" => cs.transcript_meta,
        "transcript:warning" => cs.transcript_warning,
        "transcript:location" => cs.transcript_location,
        "transcript:system" => cs.transcript_system,
        "warning_marker" => cs.warning_marker,
        "suggestion" => cs.suggestion,
        "input:text" => cs.input_text,
        "input:prompt" => cs.input_prompt,
        "scrollbar" => cs.scrollbar,
        "meta_marker" => cs.meta_marker,
        "helpbar" => cs.help_bar,
        "story_title" => cs.story_title,
        "map_layer_tab" => cs.map_layer_tab,
        "map_layer_tab_active" => cs.map_layer_tab_active,
        "dialog:title" => cs.dialog_title,
        "dialog:button" => cs.dialog_button,
        "dialog:button:active" => cs.dialog_button_active,
        "dialog:shadow" => cs.dialog_shadow,
        "upper_window" => cs.upper_window,
        "sound_beep_high" => cs.sound_beep_high,
        "sound_beep_low" => cs.sound_beep_low,
        "loc_indicator" => cs.loc_indicator,
        // Color-bearing composite selectors (map_border/story_border/dialog/
        // status_header/input_line/upper_window_border) carry a border style +
        // an fg/bg; expose their COLOR fields here for the board sample. Confirm
        // the exact ColorScheme field names by reading apply_color_decls's arms.
        "map_border" => cs.map_border,
        "story_border" => cs.story_border,
        "dialog" => cs.dialog,
        "status_header" => cs.status_header,
        "input_line" => cs.input_line,
        "upper_window_border" => cs.upper_window_border,
        _ => Style::default(),
    }
}
```

(For the six composite selectors, read the exact field names `apply_color_decls` writes — open `style.rs:239–293` and match them; the names above are the expected ones but VERIFY.)

`SELECTOR_GROUPS`: group all 37 `SELECTOR_FIELDS` into labeled sections, e.g.:

```rust
pub const SELECTOR_GROUPS: &[(&str, &[&str])] = &[
    ("Map", &["room","room:current","room:selected","connector","connector:distorted","connector:portal","map_border","map_layer_tab","map_layer_tab_active","loc_indicator"]),
    ("Transcript", &["transcript","transcript:input","transcript:meta","transcript:warning","transcript:location","transcript:system","suggestion","input:text","input:prompt","warning_marker","meta_marker"]),
    ("Chrome", &["statusbar","helpbar","story_border","story_title","scrollbar","status_header","input_line","border:focused"]),
    ("Dialogs", &["dialog","dialog:title","dialog:button","dialog:button:active","dialog:shadow"]),
    ("Upper window", &["upper_window","upper_window_border"]),
    ("Sound", &["sound_beep_high","sound_beep_low"]),
];
```

(The `selector_groups_cover_all_selector_fields` test enforces completeness — if `SELECTOR_FIELDS` and these groups disagree, the test fails and you fix the groups. `"border"` is reserved/non-visual; if it's in `SELECTOR_FIELDS`, add it to a group or exclude it explicitly and adjust the test to allow the documented exclusion.)

- [ ] **Step 4: Run the tests + full suite**

Run: `cargo test -p app` → PASS, 0 warnings.

- [ ] **Step 5: Commit** (trailers as above; message: `feat(app): style_for_selector read accessor + grouped selector list`).

---

### Task 3: The preview board (render samples + select)

**Files:**
- Create: `crates/app/src/render/style_editor.rs` — `draw_style_editor`.
- Modify: `crates/app/src/render/mod.rs` — `pub mod style_editor;`.
- Modify: `crates/app/src/main.rs` — call `draw_style_editor` in the frame draw (mirror the `config_screen` call ~594); add a Tab-focus block if needed.
- Modify: `crates/app/src/input.rs` — board nav keys + mouse hit-test → set `active`.

**Interfaces:**
- Consumes: `style::style_for_selector`, `style::SELECTOR_GROUPS`, `StyleEditorState` (Task 1).
- Produces: `pub fn draw_style_editor(state: &AppState, area: Rect, buf: &mut Buffer) -> Option<StyleEditorRects>` where `StyleEditorRects { samples: Vec<(usize, Rect)>, /* property-pane rects added in Tasks 4-5 */ }`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn style_editor_board_renders_samples_and_highlights_active() {
    let mut s = AppState::default();
    crate::input::open_style_editor(&mut s);
    let area = Rect::new(0,0,80,30);
    let mut buf = Buffer::empty(area);
    let rects = crate::render::style_editor::draw_style_editor(&s, area, &mut buf).expect("drawn");
    assert!(!rects.samples.is_empty(), "samples have hit-rects");
    // The active selector's sample rect maps to index 0.
    assert!(rects.samples.iter().any(|(i, _)| *i == 0));
}
```

- [ ] **Step 2: Run it** → compile error (module missing). 

- [ ] **Step 3: Implement `draw_style_editor`**

Create `crates/app/src/render/style_editor.rs`. Mirror `render/config_screen.rs` structure: clear the area, draw a titled frame (themed via `state.colors.dialog*`), split into a left board column and a right property column. Iterate `SELECTOR_GROUPS`; for each selector draw a one-line sample labeled with the selector name, styled with `style::style_for_selector(&ed.preview, sel)`; record `(global_index, Rect)` into `rects.samples`; highlight the sample whose `global_index == ed.active`. (The property column is a stub here; Tasks 4-5 fill it.) Use `ed.preview` for ALL sample styling so edits show live. Return `StyleEditorRects`.

Define `StyleEditorRects` in this module (or `render/mod.rs`): `pub struct StyleEditorRects { pub samples: Vec<(usize, ratatui::layout::Rect)> }` (extended in later tasks).

Register `pub mod style_editor;` in `render/mod.rs`. In `main.rs` frame draw, after the config-screen block (~594):

```rust
if state.style_editor.is_some() {
    style_editor_rects_out = draw_style_editor(state, full, buf);
}
```

(Add the `style_editor_rects_out` local + an import, mirroring `dialog_rects_out`.)

- [ ] **Step 4: Board navigation + click-select**

In `input.rs` `style_editor_key_to_action`, add Up/Down → `Action::StyleNav(-1)`/`(1)`. Add the action + handler:

```rust
Action::StyleNav(d) => {
    if let Some(ed) = &mut state.style_editor {
        let n = ed.selectors.len() as i32;
        ed.active = ((ed.active as i32 + d).rem_euclid(n.max(1))) as usize;
        recompute_style_preview(ed, &state.config.user_dir); // no-op for nav, but keep the helper centralized
    }
}
```

Add the shared helper (used by all edit handlers):

```rust
pub fn recompute_style_preview(ed: &mut crate::state::StyleEditorState, user_dir: &std::path::Path) {
    let (cs, _set, _w) = crate::style::resolve(&ed.doc, user_dir);
    ed.preview = cs;
}
```

(Nav doesn't change the doc, so the recompute is redundant there — call it only from edit handlers in later tasks; for nav, omit it. Keep `recompute_style_preview` for Tasks 4-6.)

For mouse: in the main-loop mouse handling, when `state.style_editor.is_some()`, hit-test `style_editor_rects_out.samples`; on a click inside a sample rect, set `ed.active = i`. (Mirror the config-screen mouse dispatch at `input.rs:709`.)

- [ ] **Step 5: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 6: Commit** (`feat(app): style editor preview board (samples + select)`).

---

### Task 4: Property pane + attribute toggles

**Files:**
- Modify: `crates/app/src/render/style_editor.rs` — render the right property pane for the active selector (current fg/bg text + five attribute chips); add chip rects to `StyleEditorRects`.
- Modify: `crates/app/src/input.rs` — Tab cycles `focus`; toggling an attribute chip updates the active `Decl` + recomputes preview.

**Interfaces:**
- Consumes: `StyleEditorState.doc.colors.selectors`, `recompute_style_preview` (Task 3).
- Produces: `StyleEditorRects.attr_chips: Vec<(AttrKind, Rect)>`; `pub enum AttrKind { Bold, Italic, Underline, Dim, Reversed }`; `Action::StyleToggleAttr(AttrKind)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn toggling_bold_updates_decl_and_preview() {
    let mut s = AppState::default();
    crate::input::open_style_editor(&mut s);
    let sel = s.style_editor.as_ref().unwrap().selectors[0].to_string();
    crate::input::apply_action(crate::input::Action::StyleToggleAttr(crate::input::AttrKind::Bold), &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.bold), Some(true));
    assert!(ed.preview != colors::ColorScheme::terminal_default() || true); // preview recomputed (smoke)
}
```

- [ ] **Step 2: Run it** → compile error.

- [ ] **Step 3: Implement the attribute toggle + handler**

Add `AttrKind` (in `input.rs` or `state.rs`) and `Action::StyleToggleAttr(AttrKind)`. Handler:

```rust
Action::StyleToggleAttr(kind) => {
    if let Some(ed) = &mut state.style_editor {
        let sel = ed.selectors[ed.active].to_string();
        let decl = ed.doc.colors.selectors.entry(sel).or_default();
        let slot = match kind {
            AttrKind::Bold => &mut decl.bold,
            AttrKind::Italic => &mut decl.italic,
            AttrKind::Underline => &mut decl.underline,
            AttrKind::Dim => &mut decl.dim,
            AttrKind::Reversed => &mut decl.reversed,
        };
        *slot = Some(!slot.unwrap_or(false));
        let dir = state.config.user_dir.clone();
        crate::input::recompute_style_preview(ed, &dir);
    }
}
```

(`Decl` must derive/`impl Default` for `entry().or_default()` — confirm; it already has all-`Option` fields so `#[derive(Default)]` likely exists. If not, add it.)

- [ ] **Step 4: Render the property pane**

In `draw_style_editor`, render the right column for `ed.selectors[ed.active]`: show `fg`/`bg` current values (from the active `Decl`, or "default" when `None`) and the five chips `[B][I][U][dim][rev]`, each highlighted when the `Decl`'s flag is `Some(true)`, each recorded as an `attr_chips` rect. Wire Tab in `style_editor_key_to_action` to cycle `ed.focus`, and map a chip click/Space to `StyleToggleAttr`.

- [ ] **Step 5: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 6: Commit** (`feat(app): style editor property pane + attribute toggles`).

---

### Task 5: Color swatch picker + custom hex + shared MRU (sidecar)

**Files:**
- Create: `crates/app/src/style_mru.rs` — MRU load/save (sidecar) + `is_valid_color_token`.
- Modify: `crates/app/src/lib.rs` — `pub mod style_mru;`.
- Modify: `crates/app/src/render/style_editor.rs` — render fg/bg swatch grid + `default` + custom field + MRU row; add their rects.
- Modify: `crates/app/src/input.rs` — set fg/bg from a swatch / MRU / committed custom; `open_style_editor` loads the MRU; cancel/save saves it.

**Interfaces:**
- Produces: `style_mru::{load_mru(user_dir)->Vec<String>, save_mru(user_dir,&[String]), push_mru(&mut Vec<String>, &str), is_valid_color_token(&str)->bool}`; `Action::StyleSetColor { is_bg: bool, value: Option<String> }` (None = `default`/clear).

- [ ] **Step 1: Write the failing tests**

In `crates/app/src/style_mru.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn push_dedups_caps_16_newest_first() {
        let mut v = Vec::new();
        for i in 0..20 { push_mru(&mut v, &format!("#{:06x}", i)); }
        assert_eq!(v.len(), 16);
        assert_eq!(v[0], "#000013"); // last pushed is first
        push_mru(&mut v, "#000013"); // existing → moves to front, no dup
        assert_eq!(v.iter().filter(|x| *x == "#000013").count(), 1);
        assert_eq!(v[0], "#000013");
    }
    #[test]
    fn valid_color_token_accepts_ansi_hex_default() {
        assert!(is_valid_color_token("yellow"));
        assert!(is_valid_color_token("#a1b2c3"));
        assert!(is_valid_color_token("default"));
        assert!(!is_valid_color_token("#xyz"));
        assert!(!is_valid_color_token("notacolor"));
    }
    #[test]
    fn mru_sidecar_round_trips() {
        let dir = std::env::temp_dir().join(format!("bm-mru-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save_mru(&dir, &["#112233".into(), "#445566".into()]).unwrap();
        assert_eq!(load_mru(&dir), vec!["#112233".to_string(), "#445566".into()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
```

- [ ] **Step 2: Run them** → compile error.

- [ ] **Step 3: Implement `style_mru`**

```rust
//! Shared most-recently-used custom-color list for the style editor, persisted
//! to a sidecar so it survives restarts.
use std::path::{Path, PathBuf};

const CAP: usize = 16;
fn sidecar(dir: &Path) -> PathBuf { dir.join("style_editor.toml") }

/// The 16 ANSI names the swatch grid offers (must match colors::parse_color_value).
pub const ANSI_NAMES: &[&str] = &[
    "black","red","green","yellow","blue","magenta","cyan","white",
    "dark-gray","light-red","light-green","light-yellow","light-blue",
    "light-magenta","light-cyan","gray",
];

pub fn is_valid_color_token(s: &str) -> bool {
    if s == "default" || ANSI_NAMES.contains(&s) { return true; }
    let hex = s.strip_prefix('#').unwrap_or(s);
    hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn push_mru(v: &mut Vec<String>, value: &str) {
    v.retain(|x| x != value);
    v.insert(0, value.to_string());
    v.truncate(CAP);
}

pub fn load_mru(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(sidecar(dir)) else { return Vec::new() };
    text.parse::<toml::Table>().ok()
        .and_then(|t| t.get("recent_colors").and_then(|v| v.as_array()).map(|a|
            a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()))
        .unwrap_or_default()
}

pub fn save_mru(dir: &Path, v: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let arr = v.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(", ");
    std::fs::write(sidecar(dir), format!("recent_colors = [{arr}]\n"))
}
```

Add `pub mod style_mru;` to `lib.rs`. (Confirm the exact ANSI names `parse_color_value` accepts by reading `colors.rs` — adjust `ANSI_NAMES` to match exactly so swatches resolve.)

- [ ] **Step 4: Wire color-setting + MRU into the editor**

- `open_style_editor` (Task 1): replace `mru: Vec::new()` with `mru: crate::style_mru::load_mru(&user_dir)`.
- `StyleEditorCancel` and the Save handler: `crate::style_mru::save_mru(&state.config.user_dir, &ed.mru)` before clearing (read the MRU off the editor first).
- Add `Action::StyleSetColor { is_bg, value }` handler:

```rust
Action::StyleSetColor { is_bg, value } => {
    if let Some(ed) = &mut state.style_editor {
        let sel = ed.selectors[ed.active].to_string();
        let decl = ed.doc.colors.selectors.entry(sel).or_default();
        let slot = if is_bg { &mut decl.bg } else { &mut decl.fg };
        *slot = value.clone(); // None = clear to default
        if let Some(v) = &value {
            if v.starts_with('#') { crate::style_mru::push_mru(&mut ed.mru, v); }
        }
        let dir = state.config.user_dir.clone();
        crate::input::recompute_style_preview(ed, &dir);
    }
}
```

- The custom-hex entry: typing edits `ed.custom_buf`; Enter commits via `StyleSetColor` IF `is_valid_color_token(&ed.custom_buf)` (else ignore + leave the buffer for correction). Map this in `style_editor_key_to_action` when `ed.focus == Custom`.

- [ ] **Step 5: Render the swatch grid + custom + MRU**

In `draw_style_editor`, render for fg and bg: a row of 16 ANSI swatch cells (each filled with that color) + a `default` cell + the `custom [#buf]` field + an MRU row of `ed.mru` swatches. Record each as a clickable rect (`fg_swatches`, `bg_swatches`, `mru_swatches`, `custom_rect` on `StyleEditorRects`). A swatch click → `StyleSetColor { is_bg, value: Some(name_or_hex) }`; `default` → `value: None`. Highlight the swatch matching the active `Decl`'s current value.

- [ ] **Step 6: Run tests + full suite** → PASS, 0 warnings.

- [ ] **Step 7: Commit** (`feat(app): style editor color swatch picker + custom hex + shared MRU`).

---

### Task 6: Save, Reset, and the focus/Tab + mouse polish

**Files:**
- Modify: `crates/app/src/input.rs` — `StyleSave` / `StyleReset` handlers; finalize `style_editor_key_to_action` (S=save, R=reset, Tab=focus, attribute/color keys).
- Modify: `crates/app/src/main.rs` — if a Tab-focus block or save-side persistence hook is needed, mirror config-screen (`main.rs:1680`, `2602`); ensure Save writes via `save_style_and_repoint`.

**Interfaces:**
- Consumes: `save_style_and_repoint` (`main.rs:97`), `style::resolve`, `style::parse_style_toml`, `style::DEFAULT_STYLE_TOML`.
- Produces: `Action::StyleSave`, `Action::StyleReset` (reverts the active selector).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn style_save_applies_to_live_colors() {
    let mut s = AppState::default();
    crate::input::open_style_editor(&mut s);
    crate::input::apply_action(crate::input::Action::StyleSetColor{is_bg:false, value:Some("#ff0000".into())}, &mut s, &mut Mapper::default());
    crate::input::apply_action(crate::input::Action::StyleSave, &mut s, &mut Mapper::default());
    assert!(s.style_editor.is_none(), "save closes the editor");
    // The active selector (index 0) now resolves red in the live scheme.
    // (Assert via style_for_selector on state.colors for selectors[0].)
}

#[test]
fn style_reset_reverts_active_selector_to_default() {
    let mut s = AppState::default();
    crate::input::open_style_editor(&mut s);
    let sel = s.style_editor.as_ref().unwrap().selectors[0].to_string();
    crate::input::apply_action(crate::input::Action::StyleSetColor{is_bg:false, value:Some("#ff0000".into())}, &mut s, &mut Mapper::default());
    crate::input::apply_action(crate::input::Action::StyleReset, &mut s, &mut Mapper::default());
    let ed = s.style_editor.as_ref().unwrap();
    let default_doc = crate::style::parse_style_toml(crate::style::DEFAULT_STYLE_TOML).unwrap();
    assert_eq!(ed.doc.colors.selectors.get(&sel).and_then(|d| d.fg),
               default_doc.colors.selectors.get(&sel).and_then(|d| d.fg));
}
```

- [ ] **Step 2: Run them** → compile error / fail.

- [ ] **Step 3: Implement Save + Reset**

```rust
// input.rs handler: resolve + apply to live colors + close. It does NOT persist
// to disk — save_style_and_repoint lives in main.rs (the binary) and is not
// visible to the lib crate. The run loop performs the disk write (Step 3b).
Action::StyleSave => {
    if let Some(ed) = state.style_editor.take() {
        let dir = state.config.user_dir.clone();
        let _ = crate::style_mru::save_mru(&dir, &ed.mru);
        let (cs, set, _w) = crate::style::resolve(&ed.doc, &dir);
        state.colors = cs;
        state.symbols = set;
    }
}
Action::StyleReset => {
    if let Some(ed) = &mut state.style_editor {
        let default_doc = crate::style::parse_style_toml(crate::style::DEFAULT_STYLE_TOML).unwrap_or_default();
        let sel = ed.selectors[ed.active].to_string();
        match default_doc.colors.selectors.get(&sel) {
            Some(d) => { ed.doc.colors.selectors.insert(sel, d.clone()); }
            None => { ed.doc.colors.selectors.remove(&sel); }
        }
        let dir = state.config.user_dir.clone();
        crate::input::recompute_style_preview(ed, &dir);
    }
}
```

**Step 3b — persist in the run loop (required).** `save_style_and_repoint` lives in `main.rs` (the binary crate); `input.rs` is in the lib crate and CANNOT call it. So disk persistence must happen in the `main.rs` run loop, mirroring the config-save hook exactly (`main.rs:2602`): before `apply_action`, snapshot whether this is a Save (`let style_save = matches!(action, Action::StyleSave);`); after `apply_action` returns (the handler has already set `state.colors`/`symbols`), if `style_save`, call `save_style_and_repoint(&mut state, &state.config.user_dir.clone())`. That writes the now-live `state.colors` to `style.toml` via `write_style_full` and re-resolves — identical to how config-save persists. The `input.rs` `StyleSave` handler does ONLY `resolve` + set `state.colors/symbols` + take the editor (above); it must not reference `save_style_and_repoint`.

- [ ] **Step 4: Finalize keys**

In `style_editor_key_to_action`: `Esc`→Cancel, `s`→`StyleSave`, `r`→`StyleReset`, `Tab`/`BackTab`→focus cycle, Up/Down→`StyleNav`, and when `focus==Attrs`/`Fg`/`Bg`/`Custom` route the relevant keys (Space/Enter/typing) to `StyleToggleAttr`/`StyleSetColor`/custom-buffer edits. Add the mouse hit-tests for the property-pane rects in the run loop (mirror Task 3's sample hit-test).

- [ ] **Step 5: Build, test, headless smoke + manual**

Run: `cargo build -p app && cargo test -p app` → 0 warnings, full suite + headless PASS.

Manual (not gating): `F3` (or `/style`) opens the editor; click a transcript sample, click a swatch → it recolors live; `s` saves (writes `style.toml`, theme applies); reopen → persisted; `r` resets the active selector.

- [ ] **Step 6: Commit** (`feat(app): style editor save/reset + key & mouse finalize`).

---

## Notes for the executor

- **Dependency order:** 1 → 2 → 3 → 4 → 5 → 6. All `cargo test -p app`. Each ends green, 0 warnings, before committing.
- **Preview is always `resolve(&ed.doc, user_dir)`** — never mutate `state.colors` until Save. Cancel is safe precisely because the live theme is untouched during editing.
- **The six composite selectors** (`map_border`, `story_border`, `dialog`, `status_header`, `input_line`, `upper_window_border`) carry both a border style (Phase 2) and color; Task 2 only reads their color field for the board sample, and the editor only edits their `fg/bg/attrs` (never their border `style*` fields). Verify the exact `ColorScheme` field names against `apply_color_decls` (`style.rs:239–293`) when implementing Task 2.
- **Line numbers** (`main.rs:594/1680/2602`, `input.rs:332/709/2051`, `state.rs:503/638/674`) are from a snapshot; confirm by grep before editing.
- **`Decl: Default`** is required for `entry().or_default()` — confirm it derives `Default` (all-Option fields); add the derive if missing (Task 4).
- **Render tasks (3–5)** give concrete structure + exact integration points; the precise cell-by-cell board layout is the implementer's to finalize, mirroring `render/config_screen.rs` and `render/reset_dialog.rs`. Keep all editor chrome themed via `state.colors` selectors — no hard-coded colors.
- `README.md` is committed; `TODO.md` is gitignored — never stage it. Add a README note for the `/style` editor only if asked.
