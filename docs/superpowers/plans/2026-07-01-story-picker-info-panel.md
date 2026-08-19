# Story-Picker Info Side-Panel + Row Badges — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add always-visible per-row artifact badges and a toggleable, animated info side-panel to the pre-game story picker, describing the highlighted story (header/fs metadata, blorb resource structure, saves, feature badges).

**Architecture:** Cheap byte-derived metadata (`StoryMeta`) is computed eagerly during the existing `scan_stories` pass; per-row existence badges (`RowBadges`) are computed once at picker start from picker-level context (saves dir, hint index); expensive per-file work (sibling blorb, save archive reads) is deferred lazily on highlight into a cached `StoryAux`. The panel slides in/out via `anim::Tween`, and every new UI element is themeable via `style.toml` selectors with configurable badge glyphs.

**Tech Stack:** Rust (workspace crates `app`, `blorb`); `ratatui` + `crossterm` for TUI; `zip` + `serde_json` for save archives; `toml`/`toml_edit` for config. No new dependencies.

## Global Constraints

- **No new dependencies.** `blorb`, `zip`, `toml`, `toml_edit`, `serde_json`, `ratatui`, `crossterm` are already app deps. VM crates (`zvm`/`gvm`) stay zero-dep and are untouched.
- **Cross-platform:** must run on Windows/Linux/macOS. Badge glyph defaults are ASCII single letters (`Z`/`G`/`B`/`S`/`H`), terminal-safe everywhere.
- **Styleable-UI rule:** every new UI element (panel body/border, title, label, value roles, badge cluster) must be themeable via `style.toml` selectors — no hard-coded colours. New selectors: `story_info`, `story_info:title`, `story_info:label`, `story_info:value`, `story_badge`.
- **Configurable glyphs:** the five badge glyphs live in the existing `[symbols]` config section as `String` fields with defaults matching today's ASCII, so an absent section/field is a no-op.
- **Surgical:** existing picker list rendering, sort, navigation, and the Z-machine/Glulx load paths are unchanged except for the area split, badge cluster, and new hints.
- **Panel is session-only:** always starts closed each launch; **no new config key** for open/closed state.
- **Layout constants:** `LIST_MIN_W = 24`, `PANEL_MIN_W = 28`. The panel refuses to open (toggle is a no-op) when terminal width `< LIST_MIN_W + PANEL_MIN_W`.
- **Scope:** launch picker only. No mid-game switching, no iFiction/XML parsing, no recursive scan.
- **Spec corrections (verified against source):**
  - `hints::load_hint_index(dir)` joins `"hints"` internally → pass `&cfg.user_dir`, **never** `cfg.user_dir.join("hints")`.
  - `read_archive_meta` must keep the `format_version > CURRENT_FORMAT_VERSION` rejection so future-format saves stay hidden (matching today's `list_saves` behavior).
  - `blorb::resolve_sound_blorb(path)` returns `Option<(Blorb, PathBuf)>`; `ResourceEntry.usage`/`chunk_type` are `[u8; 4]` (convert via `String::from_utf8_lossy`).

**Build/test commands** (run from repo root):
- Build: `cargo build -p app`
- Test one: `cargo test -p app <test_name>`
- Test file module: `cargo test -p app picker::tests`
- Full app tests: `cargo test -p app`

---

### Task 1: Configurable badge glyphs in `[symbols]` config

**Files:**
- Modify: `crates/app/src/config.rs` (`SymbolConfig` at ~53-72, `impl Default` at ~74-84, add free `default_badge_*` fns near line 51)

**Interfaces:**
- Produces: five new `pub` `String` fields on `SymbolConfig` — `badge_zcode`, `badge_glulx`, `badge_blorb`, `badge_save`, `badge_hint` — defaulting to `"Z"`, `"G"`, `"B"`, `"S"`, `"H"`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/config.rs`:

```rust
#[test]
fn symbol_config_badge_glyph_defaults() {
    let s = SymbolConfig::default();
    assert_eq!(s.badge_zcode, "Z");
    assert_eq!(s.badge_glulx, "G");
    assert_eq!(s.badge_blorb, "B");
    assert_eq!(s.badge_save, "S");
    assert_eq!(s.badge_hint, "H");
}

#[test]
fn symbol_config_badge_glyph_override_and_absent_default() {
    // Overriding one field parses; the others keep their defaults.
    let toml = r#"
        badge_blorb = "◆"
    "#;
    let s: SymbolConfig = toml::from_str(toml).unwrap();
    assert_eq!(s.badge_blorb, "◆");
    assert_eq!(s.badge_zcode, "Z");
    assert_eq!(s.badge_hint, "H");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app symbol_config_badge`
Expected: FAIL — `no field 'badge_zcode' on type 'SymbolConfig'`.

- [ ] **Step 3: Add the default functions**

After `default_path_style` (line ~51) in `crates/app/src/config.rs`:

```rust
pub(crate) fn default_badge_zcode() -> String { "Z".into() }
pub(crate) fn default_badge_glulx() -> String { "G".into() }
pub(crate) fn default_badge_blorb() -> String { "B".into() }
pub(crate) fn default_badge_save() -> String { "S".into() }
pub(crate) fn default_badge_hint() -> String { "H".into() }
```

- [ ] **Step 4: Add the fields to `SymbolConfig`**

In the `SymbolConfig` struct, before the `overrides` field:

```rust
    /// Row story-type badge glyph for Z-code stories (default "Z").
    #[serde(default = "default_badge_zcode")]
    pub badge_zcode: String,
    /// Row story-type badge glyph for Glulx stories (default "G").
    #[serde(default = "default_badge_glulx")]
    pub badge_glulx: String,
    /// Row "a blorb exists" artifact badge glyph (default "B").
    #[serde(default = "default_badge_blorb")]
    pub badge_blorb: String,
    /// Row "a save exists" artifact badge glyph (default "S").
    #[serde(default = "default_badge_save")]
    pub badge_save: String,
    /// Row "a hint file exists" artifact badge glyph (default "H").
    #[serde(default = "default_badge_hint")]
    pub badge_hint: String,
```

- [ ] **Step 5: Set the same defaults in `impl Default for SymbolConfig`**

In `impl Default for SymbolConfig`, before `overrides: BTreeMap::new(),`:

```rust
            badge_zcode: default_badge_zcode(),
            badge_glulx: default_badge_glulx(),
            badge_blorb: default_badge_blorb(),
            badge_save: default_badge_save(),
            badge_hint: default_badge_hint(),
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p app symbol_config_badge`
Expected: PASS (both tests).

- [ ] **Step 7: Commit**

```bash
git add crates/app/src/config.rs
git commit -m "feat(config): configurable story-picker badge glyphs in [symbols]"
```

---

### Task 2: Themeable selectors for the panel and badge cluster

**Files:**
- Modify: `crates/app/src/colors.rs` (`ColorScheme` struct ~242-303; `terminal_default()` at ~340; `from_ghostty()` at ~452)
- Modify: `crates/app/src/style.rs` (`SELECTOR_FIELDS` ~152; `SELECTOR_GROUPS` ~203; `style_for_selector` ~235; `apply_color_decls` write arms ~369)

**Interfaces:**
- Produces: `ColorScheme` fields `story_info`, `story_info_title`, `story_info_label`, `story_info_value`, `story_badge` (all `ratatui::style::Style`); selectors `story_info`, `story_info:title`, `story_info:label`, `story_info:value`, `story_badge`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/style.rs`:

```rust
#[test]
fn story_info_and_badge_selectors_are_grouped() {
    // Every selector field must appear in exactly one group (existing invariant).
    for sel in ["story_info", "story_info:title", "story_info:label",
                "story_info:value", "story_badge"] {
        assert!(SELECTOR_FIELDS.contains(&sel), "{sel} missing from SELECTOR_FIELDS");
        let count = SELECTOR_GROUPS.iter().filter(|(_, xs)| xs.contains(&sel)).count();
        assert_eq!(count, 1, "{sel} must be in exactly one group, found {count}");
    }
}

#[test]
fn story_badge_selector_reads_the_badge_style() {
    let mut cs = colors::ColorScheme::terminal_default();
    cs.story_badge = ratatui::style::Style::new()
        .fg(ratatui::style::Color::Black)
        .bg(ratatui::style::Color::Magenta);
    let got = style_for_selector(&cs, "story_badge");
    assert_eq!(got.bg, Some(ratatui::style::Color::Magenta));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p app story_info_and_badge_selectors_are_grouped story_badge_selector_reads_the_badge_style`
Expected: FAIL — `no field 'story_badge' on type 'ColorScheme'` / selector not in `SELECTOR_FIELDS`.

- [ ] **Step 3: Add the `ColorScheme` fields**

In `crates/app/src/colors.rs`, after the `pub story_title: Style,` field (~247):

```rust
    /// Story-picker info panel body + border.
    pub story_info: Style,
    /// Story-picker info panel title (story name).
    pub story_info_title: Style,
    /// Story-picker info panel field labels.
    pub story_info_label: Style,
    /// Story-picker info panel field values.
    pub story_info_value: Style,
    /// Story-picker row badge cluster (type badge + artifact letters); fg + bg.
    pub story_badge: Style,
```

- [ ] **Step 4: Initialize the fields in `terminal_default()`**

In `terminal_default()`, after `story_title: Style::new().fg(Color::White),` (~364):

```rust
            story_info: Style::new().fg(Color::Cyan),
            story_info_title: Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            story_info_label: Style::new().fg(Color::DarkGray),
            story_info_value: Style::new().fg(Color::White),
            story_badge: Style::new().fg(Color::Black).bg(Color::Cyan),
```

- [ ] **Step 5: Initialize the fields in `from_ghostty()`**

In `from_ghostty()`, after `story_title: Style::new().fg(fg),` (~539):

```rust
            story_info: Style::new().fg(scheme.palette[6]),
            story_info_title: Style::new().fg(fg).add_modifier(Modifier::BOLD),
            story_info_label: Style::new().fg(fg).add_modifier(Modifier::DIM),
            story_info_value: Style::new().fg(fg),
            story_badge: Style::new().fg(Color::Black).bg(scheme.palette[6]),
```

- [ ] **Step 6: Register the selectors in `SELECTOR_FIELDS` and `SELECTOR_GROUPS`**

In `crates/app/src/style.rs`, add to the `SELECTOR_FIELDS` array (anywhere in the list, e.g. after `"story_title",`):

```rust
    "story_info",
    "story_info:title",
    "story_info:label",
    "story_info:value",
    "story_badge",
```

Add a new group to `SELECTOR_GROUPS` (after the `"Chrome"` group):

```rust
    ("Story picker", &[
        "story_info", "story_info:title", "story_info:label",
        "story_info:value", "story_badge",
    ]),
```

- [ ] **Step 7: Add the read arms in `style_for_selector`**

In `style_for_selector`, before the composite `map_border` arms:

```rust
        "story_info"        => cs.story_info,
        "story_info:title"  => cs.story_info_title,
        "story_info:label"  => cs.story_info_label,
        "story_info:value"  => cs.story_info_value,
        "story_badge"       => cs.story_badge,
```

- [ ] **Step 8: Add the write arms in `apply_color_decls`**

In `apply_color_decls`, alongside the simple `.patch(style)` arms (e.g. near the `story_title` arm):

```rust
        "story_info"        => cs.story_info = cs.story_info.patch(style),
        "story_info:title"  => cs.story_info_title = cs.story_info_title.patch(style),
        "story_info:label"  => cs.story_info_label = cs.story_info_label.patch(style),
        "story_info:value"  => cs.story_info_value = cs.story_info_value.patch(style),
        "story_badge"       => cs.story_badge = cs.story_badge.patch(style),
```

- [ ] **Step 9: Run the new tests and the full style suite**

Run: `cargo test -p app style::tests`
Expected: PASS — new tests pass and the existing selector-completeness test still passes.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/colors.rs crates/app/src/style.rs
git commit -m "feat(style): story_info + story_badge themeable selectors"
```

---

### Task 3: Cheap `read_archive_meta` for save listings

**Files:**
- Modify: `crates/app/src/archive.rs` (add `read_archive_meta`; `load_archive` at ~343, `Meta` at ~74, `ENTRY_META` at ~36, `CURRENT_FORMAT_VERSION`)
- Modify: `crates/app/src/persist_files.rs` (`list_saves` at ~30-86, switch the per-file read)
- Test: `crates/app/src/archive.rs` tests, `crates/app/src/persist_files.rs` tests (existing must still pass)

**Interfaces:**
- Produces: `pub fn read_archive_meta(path: &Path) -> io::Result<Meta>` — reads only `meta.json`, applies the same `format_version` rejection as `load_archive`.
- Consumes (in `persist_files`): replaces `crate::archive::load_archive(&path).map(|ac| ac.meta)` with `crate::archive::read_archive_meta(&path)`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `crates/app/src/archive.rs` (reuse the existing round-trip fixture helpers already used by `save_archive`/`load_archive` tests):

```rust
#[test]
fn read_archive_meta_matches_load_archive_meta() {
    let dir = std::env::temp_dir().join(format!("lanthorn-meta-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("game.lanthorn");

    // Reuse an existing save fixture. `sample_archive_contents()` /
    // `save_archive` are already used by neighbouring tests in this module;
    // build a minimal contents value the same way they do and write it.
    let contents = sample_archive_contents();
    save_archive(&path, &contents).unwrap();

    let full = load_archive(&path).unwrap().meta;
    let quick = read_archive_meta(&path).unwrap();

    assert_eq!(quick.format_version, full.format_version);
    assert_eq!(quick.ifid, full.ifid);
    assert_eq!(quick.name, full.name);
    assert_eq!(quick.turns, full.turns);
    assert_eq!(quick.saved_at, full.saved_at);

    let _ = std::fs::remove_dir_all(&dir);
}
```

If no `sample_archive_contents()` helper exists in this module, build the `ArchiveContents` inline exactly as the nearest existing `save_archive` round-trip test does (copy its setup verbatim), then write it with `save_archive`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p app read_archive_meta_matches_load_archive_meta`
Expected: FAIL — `cannot find function 'read_archive_meta'`.

- [ ] **Step 3: Implement `read_archive_meta`**

In `crates/app/src/archive.rs`, add (near `load_archive`):

```rust
/// Read ONLY the `meta.json` entry from a save archive — avoids `load_archive`
/// unzipping the map, save image, transcript, history, screen, and aux just to
/// show a save summary. Applies the same `format_version` rejection as
/// `load_archive`, so a future-format archive is reported as an error (and thus
/// skipped by `list_saves`) exactly as today.
pub fn read_archive_meta(path: &Path) -> io::Result<Meta> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let meta: Meta = {
        let mut entry = zip.by_name(ENTRY_META).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("missing {ENTRY_META}: {e}"))
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf)?;
        serde_json::from_str(&buf).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("corrupt {ENTRY_META}: {e}"))
        })?
    };
    if meta.format_version > CURRENT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported archive format_version {}; expected <= {}",
                meta.format_version, CURRENT_FORMAT_VERSION
            ),
        ));
    }
    Ok(meta)
}
```

- [ ] **Step 4: Switch `list_saves` to the cheap read**

In `crates/app/src/persist_files.rs`, replace the per-file read block:

```rust
        // Try to read Meta from the archive; skip on failure.
        let meta = match crate::archive::load_archive(&path) {
            Ok(ac) => ac.meta,
            Err(_) => continue,
        };
```

with:

```rust
        // Read only meta.json; skip on failure (corrupt/unsupported → not listed).
        let meta = match crate::archive::read_archive_meta(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
```

- [ ] **Step 5: Run the archive + persist tests**

Run: `cargo test -p app read_archive_meta_matches_load_archive_meta`
Then: `cargo test -p app persist_files::tests`
Expected: PASS — the new test passes and all existing `list_saves` tests (`save_named_round_trip`, `list_saves_ordering_default_first`, `list_saves_skips_non_archive_files`, `delete_save_removes_file`) still pass (behavior identical, only cheaper).

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/archive.rs crates/app/src/persist_files.rs
git commit -m "perf(archive): read_archive_meta reads only meta.json; list_saves uses it"
```

---

### Task 4: `StoryMeta` eager metadata + pure header helpers

**Files:**
- Modify: `crates/app/src/picker.rs` (add types + populate in `scan_stories` at ~37-84; header helpers; tests)

**Interfaces:**
- Consumes: `hints::load_story` / `LoadedStory::{ZCode,Glulx}` / `.bytes()`; `crate::ifid::compute_ifid`; `blorb::Blorb::{is_blorb, parse, resources, has_sounds}`, `blorb::ResourceEntry`.
- Produces:
  - `pub enum Engine { ZCode, Glulx }`
  - `pub struct ChunkInfo { pub usage: String, pub number: u32, pub chunk_type: String, pub len: usize }`
  - `pub struct Features { pub sound: bool, pub graphics: bool, pub colour: Option<bool>, pub hints: bool }`
  - `pub struct StoryMeta { pub size_bytes: u64, pub modified: Option<String>, pub engine: Engine, pub format: String, pub version: Option<String>, pub serial: Option<String>, pub release: Option<u16>, pub ifid: String, pub features: Features, pub self_blorb: Option<Vec<ChunkInfo>> }`
  - `pub meta: StoryMeta` field on `StoryEntry`.
  - `pub fn chunks_of(b: &blorb::Blorb) -> Vec<ChunkInfo>` (shared with Task 6).

- [ ] **Step 1: Write failing tests for the pure header helpers**

Add to `crates/app/src/picker.rs` `mod tests` (the `minimal_v3_story()` helper already exists there):

```rust
#[test]
fn z_header_helpers_parse_version_release_serial_flags() {
    let mut b = minimal_v3_story();
    b[0x00] = 3;                       // version
    b[0x02] = 0x00; b[0x03] = 0x58;    // release 88
    b[0x12..0x18].copy_from_slice(b"840726");
    b[0x10] = 0x00; b[0x11] = 0x08 | 0x40 | 0x80; // flags2: graphics|colour|sound

    assert_eq!(z_version(&b), Some(3));
    assert_eq!(z_release(&b), Some(88));
    assert_eq!(z_serial(&b).as_deref(), Some("840726"));
    let f2 = z_flags2(&b);
    assert!(f2 & 0x0008 != 0, "graphics bit");
    assert!(f2 & 0x0040 != 0, "colour bit");
    assert!(f2 & 0x0080 != 0, "sound bit");
}

#[test]
fn glulx_version_formats_major_minor_subminor() {
    let mut b = vec![0u8; 0x40];
    b[0x00..0x04].copy_from_slice(b"Glul");
    b[0x04] = 0x00; b[0x05] = 0x03;    // major = 3
    b[0x06] = 0x01;                    // minor = 1
    b[0x07] = 0x02;                    // subminor = 2
    assert_eq!(glulx_version(&b).as_deref(), Some("3.1.2"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app picker::tests::z_header_helpers_parse_version_release_serial_flags`
Expected: FAIL — `cannot find function 'z_version'`.

- [ ] **Step 3: Add the types**

At the top of `crates/app/src/picker.rs` (after the `use` line), add:

```rust
use std::collections::HashSet;

/// The VM engine a story runs on (version-agnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    ZCode,
    Glulx,
}

/// One blorb resource-index entry, string-rendered for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkInfo {
    pub usage: String,      // "Exec" | "Pict" | "Snd " | "Data" …
    pub number: u32,
    pub chunk_type: String, // "ZCOD" | "GLUL" | "PNG " | "OGGV" …
    pub len: usize,
}

/// Best-effort static feature signals. Glulx-unknowable features are `None`/false.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Features {
    pub sound: bool,
    pub graphics: bool,
    pub colour: Option<bool>, // Z: Some(bit6); Glulx: None (runtime Glk → omit)
    pub hints: bool,          // folded in from StoryAux when the aux resolves
}

/// Eager per-story metadata, derived from bytes `scan_stories` already reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryMeta {
    pub size_bytes: u64,
    pub modified: Option<String>, // "YYYY-MM-DD"
    pub engine: Engine,
    pub format: String,           // "Z-code" | "Glulx" | "Blorb (Z-code)" | "Blorb (Glulx)"
    pub version: Option<String>,  // Z: "3"; Glulx: "3.1.2"
    pub serial: Option<String>,   // Z only
    pub release: Option<u16>,     // Z only
    pub ifid: String,
    pub features: Features,
    pub self_blorb: Option<Vec<ChunkInfo>>, // Some when the story file itself is a blorb
}
```

- [ ] **Step 4: Add the pure helpers**

Add to `crates/app/src/picker.rs` (module scope, above `#[cfg(test)]`):

```rust
/// Z-machine version byte at header offset 0x00.
fn z_version(exec: &[u8]) -> Option<u8> {
    exec.first().copied()
}

/// Z-machine release: big-endian word at header offset 0x02.
fn z_release(exec: &[u8]) -> Option<u16> {
    match (exec.get(0x02), exec.get(0x03)) {
        (Some(&h), Some(&l)) => Some(u16::from_be_bytes([h, l])),
        _ => None,
    }
}

/// Z-machine serial: 6 ASCII bytes at header offset 0x12..0x18.
fn z_serial(exec: &[u8]) -> Option<String> {
    let s = exec.get(0x12..0x18)?;
    Some(String::from_utf8_lossy(s).into_owned())
}

/// Z-machine Flags2: big-endian word at header offset 0x10.
/// bit 3 (0x0008)=graphics, bit 6 (0x0040)=colours, bit 7 (0x0080)=sound.
fn z_flags2(exec: &[u8]) -> u16 {
    match (exec.get(0x10), exec.get(0x11)) {
        (Some(&h), Some(&l)) => u16::from_be_bytes([h, l]),
        _ => 0,
    }
}

/// Glulx version: 16-bit major at 0x04, minor at 0x06, subminor at 0x07 →
/// "major.minor.subminor".
fn glulx_version(exec: &[u8]) -> Option<String> {
    let major = u16::from_be_bytes([*exec.get(0x04)?, *exec.get(0x05)?]);
    let minor = *exec.get(0x06)?;
    let subminor = *exec.get(0x07)?;
    Some(format!("{major}.{minor}.{subminor}"))
}

/// Convert a parsed blorb's resource index into displayable `ChunkInfo`.
pub fn chunks_of(b: &blorb::Blorb) -> Vec<ChunkInfo> {
    b.resources()
        .iter()
        .map(|r| ChunkInfo {
            usage: String::from_utf8_lossy(&r.usage).into_owned(),
            number: r.number,
            chunk_type: String::from_utf8_lossy(&r.chunk_type).into_owned(),
            len: r.len,
        })
        .collect()
}

/// Eager `Features` for a Z-code exec image, folding in self-blorb resources.
fn z_features(exec: &[u8], self_blorb: Option<&[ChunkInfo]>) -> Features {
    let f2 = z_flags2(exec);
    let mut sound = f2 & 0x0080 != 0;
    let mut graphics = f2 & 0x0008 != 0;
    if let Some(chunks) = self_blorb {
        if chunks.iter().any(|c| c.usage == "Snd ") {
            sound = true;
        }
        if chunks.iter().any(|c| c.usage == "Pict") {
            graphics = true;
        }
    }
    Features { sound, graphics, colour: Some(f2 & 0x0040 != 0), hints: false }
}

/// Eager `Features` for a Glulx story — colour is runtime Glk (None); sound and
/// graphics come from a self-blorb only.
fn glulx_features(self_blorb: Option<&[ChunkInfo]>) -> Features {
    let mut f = Features { sound: false, graphics: false, colour: None, hints: false };
    if let Some(chunks) = self_blorb {
        f.sound = chunks.iter().any(|c| c.usage == "Snd ");
        f.graphics = chunks.iter().any(|c| c.usage == "Pict");
    }
    f
}
```

- [ ] **Step 5: Run the helper tests to verify they pass**

Run: `cargo test -p app picker::tests::z_header_helpers_parse_version_release_serial_flags picker::tests::glulx_version_formats_major_minor_subminor`
Expected: PASS.

- [ ] **Step 6: Add the `meta` field to `StoryEntry` and populate it in `scan_stories`**

Add the field to `StoryEntry`:

```rust
pub struct StoryEntry {
    pub path: PathBuf,
    pub title: String,
    pub filename: String,
    pub meta: StoryMeta,
}
```

In `scan_stories`, replace the tail of the loop body (from the `let ifid = …` line through `out.push(...)`) with the following. It reuses the already-loaded `loaded`/`bytes`/`ifid` and reads the raw file only for blorb-extension files (self-blorb chunk enumeration needs the container index, which extraction discards):

```rust
        let ifid = crate::ifid::compute_ifid(&bytes);
        let title = crate::session::known_title(&ifid)
            .map(|t| t.to_string())
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&filename)
                    .to_string()
            });

        // fs metadata: size + mtime → "YYYY-MM-DD".
        let fs_meta = std::fs::metadata(&path).ok();
        let size_bytes = fs_meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified = fs_meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(format_mtime_ymd);

        // Self-blorb chunks: only blorb-container files carry a resource index,
        // and extraction (`load_story`) discards it — re-read the raw file for
        // those extensions only, so plain .z* files stay single-read.
        let self_blorb = if is_blorb_ext(&path) {
            std::fs::read(&path).ok().and_then(|raw| {
                if blorb::Blorb::is_blorb(&raw) {
                    blorb::Blorb::parse(raw).ok().map(|b| chunks_of(&b))
                } else {
                    None
                }
            })
        } else {
            None
        };

        let engine = match &loaded {
            crate::hints::LoadedStory::ZCode(_) => Engine::ZCode,
            crate::hints::LoadedStory::Glulx(_) => Engine::Glulx,
        };
        let is_container = self_blorb.is_some();
        let (version, serial, release, features, format) = match engine {
            Engine::ZCode => {
                let version = z_version(&bytes).map(|v| v.to_string());
                let serial = z_serial(&bytes);
                let release = z_release(&bytes);
                let features = z_features(&bytes, self_blorb.as_deref());
                let format = if is_container { "Blorb (Z-code)" } else { "Z-code" };
                (version, serial, release, features, format.to_string())
            }
            Engine::Glulx => {
                let version = glulx_version(&bytes);
                let features = glulx_features(self_blorb.as_deref());
                let format = if is_container { "Blorb (Glulx)" } else { "Glulx" };
                (version, None, None, features, format.to_string())
            }
        };

        let meta = StoryMeta {
            size_bytes,
            modified,
            engine,
            format,
            version,
            serial,
            release,
            ifid,
            features,
            self_blorb,
        };
        out.push(StoryEntry { path, title, filename, meta });
```

- [ ] **Step 7: Add the small `scan_stories` support helpers**

Add near the existing `has_story_ext`:

```rust
/// True for blorb-container extensions (case-insensitive).
fn is_blorb_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zblorb" | "blorb" | "gblorb" | "blb"))
        .unwrap_or(false)
}

/// Format a `SystemTime` mtime as "YYYY-MM-DD" (UTC, civil-date arithmetic; no
/// chrono dependency). Returns None if the time is before the Unix epoch.
fn format_mtime_ymd(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!("{y:04}-{m:02}-{d:02}"))
}
```

- [ ] **Step 8: Update existing `scan_stories` tests to the new `StoryEntry` shape**

The existing tests construct `StoryEntry` only via `scan_stories` (they read `.filename`/`.title`), so they compile unchanged. Add a coverage test:

```rust
#[test]
fn scan_populates_story_meta_for_v3() {
    let dir = temp_dir("meta");
    let mut b = minimal_v3_story();
    b[0x02] = 0x00; b[0x03] = 0x58;                 // release 88
    b[0x12..0x18].copy_from_slice(b"840726");
    b[0x10] = 0x00; b[0x11] = 0x40;                 // colour bit set
    std::fs::write(dir.join("game.z3"), &b).unwrap();

    let stories = scan_stories(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(stories.len(), 1);
    let m = &stories[0].meta;
    assert_eq!(m.engine, Engine::ZCode);
    assert_eq!(m.format, "Z-code");
    assert_eq!(m.version.as_deref(), Some("3"));
    assert_eq!(m.release, Some(88));
    assert_eq!(m.serial.as_deref(), Some("840726"));
    assert_eq!(m.features.colour, Some(true));
    assert!(m.size_bytes > 0);
    assert!(m.self_blorb.is_none());
}
```

- [ ] **Step 9: Run the picker tests**

Run: `cargo test -p app picker::tests`
Expected: PASS — all existing tests plus the new `scan_populates_story_meta_for_v3` and helper tests.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/picker.rs
git commit -m "feat(picker): eager StoryMeta + pure Z/Glulx header helpers"
```

---

### Task 5: `RowBadges` + `compute_row_badges`

**Files:**
- Modify: `crates/app/src/picker.rs` (add `RowBadges`, `compute_row_badges`, `BadgeGlyphs`; tests)

**Interfaces:**
- Consumes: `StoryEntry.meta` (Task 4); `hints::HintIndex::get`.
- Produces:
  - `pub struct RowBadges { pub blorb: bool, pub save: bool, pub hint: bool }`
  - `pub fn compute_row_badges(entry: &StoryEntry, save_names: &HashSet<String>, hint_index: &hints::HintIndex) -> RowBadges`
  - `pub struct BadgeGlyphs<'a> { pub zcode, glulx, blorb, save, hint: &'a str }` + `BadgeGlyphs::from_symbols(&crate::config::SymbolConfig)`.

- [ ] **Step 1: Write the failing table-driven test**

Add to `crates/app/src/picker.rs` `mod tests`:

```rust
// Build a StoryEntry with a controllable ifid + self_blorb, on a synthetic path.
fn entry_with(ifid: &str, path: PathBuf, self_blorb: Option<Vec<ChunkInfo>>) -> StoryEntry {
    StoryEntry {
        path,
        title: "T".into(),
        filename: "t.z5".into(),
        meta: StoryMeta {
            size_bytes: 1, modified: None, engine: Engine::ZCode,
            format: "Z-code".into(), version: Some("5".into()),
            serial: None, release: None, ifid: ifid.into(),
            features: Features::default(), self_blorb,
        },
    }
}

#[test]
fn compute_row_badges_covers_each_signal() {
    let dir = temp_dir("badges");
    // A self-blorb story lights `blorb` with no sibling.
    let e_self = entry_with("IFID-A", dir.join("a.z5"),
        Some(vec![ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 4 }]));
    // A story with a same-stem sibling .blorb lights `blorb`.
    std::fs::write(dir.join("b.z5"), b"x").unwrap();
    std::fs::write(dir.join("b.blorb"), b"x").unwrap();
    let e_sibling = entry_with("IFID-B", dir.join("b.z5"), None);
    // A plain story with nothing.
    let e_bare = entry_with("IFID-C", dir.join("c.z5"), None);

    let mut save_names = HashSet::new();
    save_names.insert("IFID-A.lanthorn".to_string());          // default save for A
    save_names.insert("IFID-B-before.lanthorn".to_string());   // named save for B

    let hi = hints::load_hint_index(&dir); // empty index (no hints/index.toml)

    let a = compute_row_badges(&e_self, &save_names, &hi);
    let b = compute_row_badges(&e_sibling, &save_names, &hi);
    let c = compute_row_badges(&e_bare, &save_names, &hi);
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!((a.blorb, a.save, a.hint), (true, true, false));
    assert_eq!((b.blorb, b.save, b.hint), (true, true, false));
    assert_eq!((c.blorb, c.save, c.hint), (false, false, false));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app picker::tests::compute_row_badges_covers_each_signal`
Expected: FAIL — `cannot find struct 'RowBadges'`.

- [ ] **Step 3: Implement `RowBadges` + helper + `BadgeGlyphs`**

Add to `crates/app/src/picker.rs` (module scope):

```rust
/// Cheap existence flags shown on every list row (panel-independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RowBadges {
    pub blorb: bool,
    pub save: bool,
    pub hint: bool,
}

/// True if a same-stem `.blb`/`.blorb`/`.zblorb` sibling of `path` exists.
fn sibling_blorb_exists(path: &Path) -> bool {
    ["blb", "blorb", "zblorb"].iter().any(|ext| {
        let cand = path.with_extension(ext);
        cand != *path && cand.exists()
    })
}

/// Compute a row's artifact badges. `save_names` is the saves-dir listing read
/// once; `hint_index` is loaded once at picker start. No archive reads.
pub fn compute_row_badges(
    entry: &StoryEntry,
    save_names: &HashSet<String>,
    hint_index: &hints::HintIndex,
) -> RowBadges {
    let ifid = &entry.meta.ifid;
    RowBadges {
        blorb: entry.meta.self_blorb.is_some() || sibling_blorb_exists(&entry.path),
        save: save_names.iter().any(|n| n.starts_with(ifid.as_str())),
        hint: hint_index.get(ifid).is_some(),
    }
}

/// Borrowed badge glyphs from the `[symbols]` config, for row rendering.
pub struct BadgeGlyphs<'a> {
    pub zcode: &'a str,
    pub glulx: &'a str,
    pub blorb: &'a str,
    pub save: &'a str,
    pub hint: &'a str,
}

impl<'a> BadgeGlyphs<'a> {
    pub fn from_symbols(s: &'a crate::config::SymbolConfig) -> Self {
        Self {
            zcode: &s.badge_zcode,
            glulx: &s.badge_glulx,
            blorb: &s.badge_blorb,
            save: &s.badge_save,
            hint: &s.badge_hint,
        }
    }
}
```

Ensure `hints` is reachable from `picker.rs` (it already calls `crate::hints::load_story`; add `use crate::hints;` at the top if not already imported, or fully-qualify as `crate::hints`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app picker::tests::compute_row_badges_covers_each_signal`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/picker.rs
git commit -m "feat(picker): RowBadges + compute_row_badges + BadgeGlyphs"
```

---

### Task 6: `StoryAux` + lazy `resolve_aux`

**Files:**
- Modify: `crates/app/src/picker.rs` (add `StoryAux`, `resolve_aux`; tests)

**Interfaces:**
- Consumes: `blorb::resolve_sound_blorb(path) -> Option<(Blorb, PathBuf)>`; `chunks_of` (Task 4); `persist_files::list_saves(dir, ifid)`; `hints::HintIndex::get`.
- Produces:
  - `pub struct StoryAux { pub assoc_blorb: Option<(PathBuf, Vec<ChunkInfo>)>, pub saves: Vec<crate::persist_files::SaveInfo>, pub hints_available: bool }`
  - `pub fn resolve_aux(entry: &StoryEntry, save_dir: &Path, hint_index: &hints::HintIndex) -> StoryAux`

- [ ] **Step 1: Write the failing test**

Add to `crates/app/src/picker.rs` `mod tests` (uses the blorb builder shape from the crate's tests — a minimal single-`Snd ` blorb so `resolve_sound_blorb` accepts the sibling):

```rust
// Minimal blorb with one Snd resource so resolve_sound_blorb accepts a sibling.
fn blorb_with_sound() -> Vec<u8> {
    fn chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(ty);
        v.extend_from_slice(&(data.len() as u32).to_be_bytes());
        v.extend_from_slice(data);
        if data.len() % 2 == 1 { v.push(0); }
        v
    }
    let ridx_data_len = 4 + 12;
    let snd_off = 12 + 8 + ridx_data_len + (ridx_data_len % 2);
    let mut ridx = Vec::new();
    ridx.extend_from_slice(&1u32.to_be_bytes());
    ridx.extend_from_slice(b"Snd ");
    ridx.extend_from_slice(&0u32.to_be_bytes());
    ridx.extend_from_slice(&(snd_off as u32).to_be_bytes());
    let mut inner = Vec::new();
    inner.extend_from_slice(b"IFRS");
    inner.extend_from_slice(&chunk(b"RIdx", &ridx));
    inner.extend_from_slice(&chunk(b"OGGV", b"snd"));
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
    file.extend_from_slice(&inner);
    file
}

#[test]
fn resolve_aux_finds_sibling_blorb_and_saves() {
    let dir = temp_dir("aux");
    std::fs::write(dir.join("g.z5"), minimal_v3_story()).unwrap();
    std::fs::write(dir.join("g.blb"), blorb_with_sound()).unwrap();
    let entry = entry_with("IFID-G", dir.join("g.z5"), None);

    let hi = hints::load_hint_index(&dir);
    let aux = resolve_aux(&entry, &dir, &hi); // save_dir=dir (no saves present)
    let _ = std::fs::remove_dir_all(&dir);

    let (src, chunks) = aux.assoc_blorb.expect("sibling blorb resolved");
    assert!(src.ends_with("g.blb"));
    assert!(chunks.iter().any(|c| c.usage == "Snd "));
    assert!(aux.saves.is_empty());
    assert!(!aux.hints_available);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app picker::tests::resolve_aux_finds_sibling_blorb_and_saves`
Expected: FAIL — `cannot find struct 'StoryAux'`.

- [ ] **Step 3: Implement `StoryAux` + `resolve_aux`**

Add to `crates/app/src/picker.rs` (module scope):

```rust
/// Lazily-resolved, per-highlight data that touches other files/dirs.
pub struct StoryAux {
    /// Sibling/dir-scan blorb resources when the story is NOT itself a blorb.
    /// Carries the source path so the panel can name the file.
    pub assoc_blorb: Option<(PathBuf, Vec<ChunkInfo>)>,
    pub saves: Vec<crate::persist_files::SaveInfo>,
    pub hints_available: bool,
}

/// Resolve the lazy aux for one story. `save_dir` is `user_dir/saves`;
/// `hint_index` is the shared index loaded once at picker start.
pub fn resolve_aux(
    entry: &StoryEntry,
    save_dir: &Path,
    hint_index: &hints::HintIndex,
) -> StoryAux {
    // Only record an ASSOCIATED blorb (a different file); the self-blorb case is
    // already carried in StoryMeta.self_blorb.
    let assoc_blorb = match blorb::resolve_sound_blorb(&entry.path) {
        Some((b, src)) if src != entry.path => Some((src, chunks_of(&b))),
        _ => None,
    };
    let saves = crate::persist_files::list_saves(save_dir, &entry.meta.ifid);
    let hints_available = hint_index.get(&entry.meta.ifid).is_some();
    StoryAux { assoc_blorb, saves, hints_available }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p app picker::tests::resolve_aux_finds_sibling_blorb_and_saves`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/app/src/picker.rs
git commit -m "feat(picker): lazy StoryAux + resolve_aux"
```

---

### Task 7: Render row badges in the picker

**Files:**
- Modify: `crates/app/src/main.rs` (`run_story_picker` ~854; `draw_story_picker` ~950)

**Interfaces:**
- Consumes: `picker::{RowBadges, BadgeGlyphs, compute_row_badges, Engine}`; `hints::load_hint_index`; `cs.story_badge`; `cfg.symbols`, `cfg.user_dir`.
- Produces: `draw_story_picker` gains `badges: &[picker::RowBadges]` and `glyphs: &picker::BadgeGlyphs` parameters and draws a right-aligned badge cluster per row.

- [ ] **Step 1: Write the failing render tests**

Add to the render tests module in `crates/app/src/main.rs` (the module already has picker render tests; reuse its `Buffer`/`Rect` fixture idioms). Build entries via `picker::scan_stories` on a temp dir, or construct minimal `StoryEntry` values if a test constructor is available. Use a buffer-scan helper to read a row's rendered text:

```rust
#[test]
fn row_renders_type_badge_and_present_artifacts() {
    use ratatui::{buffer::Buffer, layout::Rect};
    let cs = app::colors::ColorScheme::terminal_default();
    let sym = app::config::SymbolConfig::default();
    let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

    // One Z-code story with all three artifacts, one Glulx story with only a save.
    let stories = make_two_test_stories(); // helper below
    let badges = vec![
        app::picker::RowBadges { blorb: true, save: true, hint: true },
        app::picker::RowBadges { blorb: false, save: true, hint: false },
    ];
    let mut list = app::list_scroll::ListScroll::new();
    list.len(stories.len());

    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    let dir = std::path::Path::new("/tmp");
    draw_story_picker(&stories, &list, &badges, &glyphs, dir, &cs, area, &mut buf);

    let row0 = row_text(&buf, 2, area);  // list starts at area.y + 2
    let row1 = row_text(&buf, 3, area);
    assert!(row0.contains("Z B S H"), "got: {row0:?}");
    assert!(row1.contains("G S"), "got: {row1:?}");
    assert!(!row1.contains("G B"), "absent artifacts omitted: {row1:?}");
}

#[test]
fn row_uses_configured_badge_glyphs() {
    use ratatui::{buffer::Buffer, layout::Rect};
    let cs = app::colors::ColorScheme::terminal_default();
    let mut sym = app::config::SymbolConfig::default();
    sym.badge_zcode = "z!".into();
    sym.badge_blorb = "◆".into();
    let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

    let stories = make_two_test_stories();
    let badges = vec![
        app::picker::RowBadges { blorb: true, save: false, hint: false },
        app::picker::RowBadges::default(),
    ];
    let mut list = app::list_scroll::ListScroll::new();
    list.len(stories.len());
    let area = Rect::new(0, 0, 60, 10);
    let mut buf = Buffer::empty(area);
    draw_story_picker(&stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
                      &cs, area, &mut buf);
    let row0 = row_text(&buf, 2, area);
    assert!(row0.contains("z! ◆"), "configured glyphs used: {row0:?}");
}
```

Add the two small test helpers to the same module (if `row_text`/`make_two_test_stories` don't already exist). `make_two_test_stories` builds a Z-code and a Glulx `StoryEntry` with the minimal `StoryMeta` (only `engine`/`ifid` matter for row rendering); `row_text` scans a buffer row into a `String`:

```rust
fn row_text(buf: &ratatui::buffer::Buffer, y: u16, area: ratatui::layout::Rect) -> String {
    (area.left()..area.right())
        .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
        .collect()
}

fn make_two_test_stories() -> Vec<app::picker::StoryEntry> {
    use app::picker::{StoryEntry, StoryMeta, Engine, Features};
    let mk = |title: &str, engine: Engine| StoryEntry {
        path: std::path::PathBuf::from(format!("/tmp/{title}.z5")),
        title: title.into(),
        filename: format!("{title}.z5"),
        meta: StoryMeta {
            size_bytes: 1, modified: None, engine, format: "Z-code".into(),
            version: None, serial: None, release: None, ifid: title.into(),
            features: Features::default(), self_blorb: None,
        },
    };
    vec![mk("Zork", Engine::ZCode), mk("Anchorhead", Engine::Glulx)]
}
```

(If `StoryEntry`/`StoryMeta` fields aren't constructible from `main.rs` tests, add a `#[cfg(test)] pub fn StoryEntry::for_test(...)` constructor to `picker.rs` in this task and use it — but they are all `pub`, so direct construction compiles.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app row_renders_type_badge_and_present_artifacts`
Expected: FAIL — `draw_story_picker` takes 6 args, not 8.

- [ ] **Step 3: Change `draw_story_picker`'s signature and render the cluster**

Update the signature:

```rust
fn draw_story_picker(
    stories: &[app::picker::StoryEntry],
    list: &app::list_scroll::ListScroll,
    badges: &[app::picker::RowBadges],
    glyphs: &app::picker::BadgeGlyphs,
    dir: &std::path::Path,
    cs: &app::colors::ColorScheme,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) -> (Vec<(usize, Rect)>, usize) {
```

Inside the per-row loop, after the existing `draw_str_clipped(buf, area.x, y, &line, style, row_rect);`, append the badge cluster (right-aligned within `row_w`, styled `story_badge`):

```rust
        // Right-aligned badge cluster: type glyph then present artifact glyphs.
        let b = badges.get(i).copied().unwrap_or_default();
        let type_glyph = match entry.meta.engine {
            app::picker::Engine::ZCode => glyphs.zcode,
            app::picker::Engine::Glulx => glyphs.glulx,
        };
        let mut cluster: Vec<&str> = vec![type_glyph];
        if b.blorb { cluster.push(glyphs.blorb); }
        if b.save { cluster.push(glyphs.save); }
        if b.hint { cluster.push(glyphs.hint); }
        let cluster_str = cluster.join(" ");
        let cluster_w = cluster_str.chars().count() as u16;
        if cluster_w + 1 < row_w {
            let bx = area.left() + row_w - cluster_w;
            draw_str_clipped(buf, bx, y, &cluster_str, cs.story_badge, row_rect);
        }
```

`RowBadges` derives `Default`, so `unwrap_or_default()` compiles.

- [ ] **Step 4: Build the badges vec and pass the new args in `run_story_picker`**

In `run_story_picker`, after `let stories = app::picker::scan_stories(dir);` and the empty check, add:

```rust
    // Row badges: one saves-dir readdir + one shared hint index, computed once.
    let save_dir = dir_saves(&cfg.user_dir);
    let save_names: std::collections::HashSet<String> = std::fs::read_dir(&save_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    let hint_index = app::hints::load_hint_index(&cfg.user_dir);
    let row_badges: Vec<app::picker::RowBadges> = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, &save_names, &hint_index))
        .collect();
    let badge_glyphs = app::picker::BadgeGlyphs::from_symbols(&cfg.symbols);
```

Where `dir_saves` is the existing `saves_dir` helper (`fn saves_dir(user_dir) -> user_dir.join("saves")`) — call `saves_dir(&cfg.user_dir)` directly (rename in the snippet above to `saves_dir`).

Update the `draw_story_picker` call inside the draw closure:

```rust
            let (rects, vp) =
                draw_story_picker(&stories, &list, &row_badges, &badge_glyphs, dir, &cs, area, buf);
```

- [ ] **Step 5: Run the render tests**

Run: `cargo test -p app row_renders_type_badge_and_present_artifacts row_uses_configured_badge_glyphs`
Then: `cargo test -p app` (ensure the whole app crate still compiles and passes).
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/main.rs crates/app/src/picker.rs
git commit -m "feat(picker): render per-row story-type + artifact badges"
```

---

### Task 8: `draw_info_panel` — the panel content renderer

**Files:**
- Modify: `crates/app/src/main.rs` (add `draw_info_panel`; reuse `render::paneframe` border helper + `draw_str_clipped`)

**Interfaces:**
- Consumes: `picker::{StoryMeta, StoryAux, ChunkInfo, Features}`; `cs.story_info`, `cs.story_info_title`, `cs.story_info_label`, `cs.story_info_value`; the existing pane-border drawing helper used for dialogs/story pane.
- Produces: `fn draw_info_panel(meta: &picker::StoryMeta, aux: Option<&picker::StoryAux>, area: Rect, cs: &colors::ColorScheme, buf: &mut Buffer)`.

- [ ] **Step 1: Write the failing test**

Add to the render tests in `crates/app/src/main.rs`:

```rust
#[test]
fn info_panel_renders_metadata_features_and_resources() {
    use ratatui::{buffer::Buffer, layout::Rect};
    let cs = app::colors::ColorScheme::terminal_default();
    let meta = app::picker::StoryMeta {
        size_bytes: 92 * 1024,
        modified: Some("2026-06-30".into()),
        engine: app::picker::Engine::ZCode,
        format: "Z-code".into(),
        version: Some("3".into()),
        serial: Some("840726".into()),
        release: Some(88),
        ifid: "ZCODE-88-840726".into(),
        features: app::picker::Features { sound: true, graphics: true, colour: Some(false), hints: true },
        self_blorb: Some(vec![
            app::picker::ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 92 * 1024 },
        ]),
    };
    let area = Rect::new(0, 0, 34, 20);
    let mut buf = Buffer::empty(area);
    draw_info_panel(&meta, None, area, &cs, &mut buf);

    let text = buffer_to_string(&buf, area); // whole-buffer scan helper
    assert!(text.contains("Z-code"), "format line: {text:?}");
    assert!(text.contains("Release 88"));
    assert!(text.contains("840726"));
    assert!(text.contains("ZCODE-88-840726"));
    assert!(text.contains("sound"));
    assert!(text.contains("graphics"));
    assert!(text.contains("hints"));
    assert!(text.contains("Exec"));
    assert!(text.contains("ZCOD"));
}
```

Add a `buffer_to_string` helper if one doesn't exist in the module:

```rust
fn buffer_to_string(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> String {
    let mut out = String::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell((x, y)) { out.push_str(c.symbol()); }
        }
        out.push('\n');
    }
    out
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app info_panel_renders_metadata_features_and_resources`
Expected: FAIL — `cannot find function 'draw_info_panel'`.

- [ ] **Step 3: Implement `draw_info_panel`**

Add to `crates/app/src/main.rs`. Draw a background fill + border with `cs.story_info`, a title line with `cs.story_info_title`, then labelled value lines (`cs.story_info_label` prefix + `cs.story_info_value`), the features line, and the resources/saves sections. Clip each line to the inner width; clip the whole panel to `area` height (draw a scrollbar column only if content overflows — see step 4).

```rust
fn draw_info_panel(
    meta: &app::picker::StoryMeta,
    aux: Option<&app::picker::StoryAux>,
    area: Rect,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(cs.story_info);
            }
        }
    }
    // Single-line border box titled " Info " using the shared pane border helper.
    // (Use the same border-draw routine the dialog/story pane uses; e.g.
    // app::render::paneframe::draw_box(buf, area, BorderStyle::Single,
    // PaneSides::all(BorderStyle::Single), &PaneGlyphs::default(), cs.story_info);
    // then write the " Info " title into the top border.)
    draw_panel_border(buf, area, " Info ", cs); // small local helper, step 3b

    let inner = Rect::new(area.x + 1, area.y + 1, area.width.saturating_sub(2), area.height.saturating_sub(2));
    let mut lines: Vec<(String, ratatui::style::Style)> = Vec::new();

    // Title.
    lines.push((title_of(meta), cs.story_info_title));
    // filename · size · modified.
    let mut fs_line = format!("{} · {}", panel_filename(meta), human_size(meta.size_bytes));
    if let Some(m) = &meta.modified { fs_line.push_str(&format!(" · {m}")); }
    lines.push((fs_line, cs.story_info_value));
    // format + version · release.
    let mut fmt_line = meta.format.clone();
    if let Some(v) = &meta.version {
        fmt_line = match meta.engine {
            app::picker::Engine::ZCode => format!("{} v{}", meta.format, v),
            app::picker::Engine::Glulx => format!("{} {}", meta.format, v),
        };
    }
    if let Some(r) = meta.release { fmt_line.push_str(&format!(" · Release {r}")); }
    lines.push((fmt_line, cs.story_info_value));
    // serial (Z only).
    if let Some(s) = &meta.serial { lines.push((format!("Serial {s}"), cs.story_info_value)); }
    // ifid.
    lines.push((format!("IFID {}", meta.ifid), cs.story_info_value));
    // features line (present badges only).
    let feats = feature_words(&meta.features, aux);
    if !feats.is_empty() {
        lines.push((format!("Features: {}", feats.join(" ")), cs.story_info_value));
    }

    // Resources: self_blorb, else aux.assoc_blorb.
    let (res_header, chunks): (Option<String>, &[app::picker::ChunkInfo]) =
        if let Some(c) = &meta.self_blorb {
            (Some(format!("Resources ({})", panel_filename(meta))), c.as_slice())
        } else if let Some((src, c)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
            let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("blorb");
            (Some(format!("Resources ({name})")), c.as_slice())
        } else {
            (None, &[])
        };
    if let Some(h) = res_header {
        lines.push((String::new(), cs.story_info_value));
        lines.push((h, cs.story_info_label));
        for c in chunks {
            lines.push((format!(" {:<4} #{} {:<4} {}", c.usage, c.number, c.chunk_type, human_size(c.len as u64)),
                        cs.story_info_value));
        }
    }

    // Saves.
    if let Some(saves) = aux.map(|a| &a.saves) {
        if !saves.is_empty() {
            lines.push((String::new(), cs.story_info_value));
            lines.push((format!("Saves ({})", saves.len()), cs.story_info_label));
            for s in saves {
                let when = s.saved_at.get(0..10).unwrap_or(&s.saved_at);
                lines.push((format!(" {}  turn {} · {}", s.name, s.turns, when), cs.story_info_value));
            }
        }
    }

    // Clip to inner height (scrollbar handled in step 4).
    for (i, (text, style)) in lines.iter().enumerate() {
        if i as u16 >= inner.height { break; }
        let y = inner.y + i as u16;
        draw_str_clipped(buf, inner.x, y, text, *style, inner);
    }
}
```

- [ ] **Step 3b: Add the small local helpers**

Add near `draw_info_panel` (reuse existing utilities where they exist — e.g. a `human_size` may already exist; if so, use it and drop this copy):

```rust
fn title_of(meta: &app::picker::StoryMeta) -> String {
    // The picker already resolves a title on the entry; the panel title mirrors it.
    // Fall back to the ifid if somehow empty.
    if meta.ifid.is_empty() { "Story".into() } else { meta.ifid.clone() }
}

fn panel_filename(_meta: &app::picker::StoryMeta) -> String {
    // Filled by the caller via a dedicated field if needed; see note below.
    String::new()
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

fn feature_words(f: &app::picker::Features, aux: Option<&app::picker::StoryAux>) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut sound = f.sound;
    let mut graphics = f.graphics;
    if let Some((_, chunks)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
        if chunks.iter().any(|c| c.usage == "Snd ") { sound = true; }
        if chunks.iter().any(|c| c.usage == "Pict") { graphics = true; }
    }
    if sound { v.push("sound"); }
    if graphics { v.push("graphics"); }
    if f.colour == Some(true) { v.push("colour"); }
    if f.hints || aux.map(|a| a.hints_available).unwrap_or(false) { v.push("hints"); }
    v
}
```

**Note on title/filename:** `StoryMeta` has no `title`/`filename` (those live on `StoryEntry`). Rather than the placeholder helpers above, change `draw_info_panel` to also take the entry's `title: &str` and `filename: &str` (the caller in Task 9 has the `StoryEntry`). Update the signature to `draw_info_panel(title, filename, meta, aux, area, cs, buf)` and use `title` for the title line and `filename` for `panel_filename`. Delete the `title_of`/`panel_filename` stubs. (This keeps `StoryMeta` free of display strings.)

- [ ] **Step 3c: Update the test for the final signature**

Adjust the Step-1 test call to `draw_info_panel("Zork I", "zork1.z3", &meta, None, area, &cs, &mut buf)` and add `assert!(text.contains("Zork I"))` and `assert!(text.contains("zork1.z3"))`.

- [ ] **Step 4: Add overflow scrollbar (only when content exceeds height)**

When `lines.len() as u16 > inner.height`, reserve the last inner column for a scrollbar and draw it with the shared helper `app::render::scroll::draw_scrollbar(buf, sb_area, lines.len(), inner.height as usize, 0, cs.scrollbar)` (top-anchored; the panel content is short and does not need live scroll state — this just signals overflow). Clip text width to `inner.width - 1` in that case.

- [ ] **Step 5: Run the test**

Run: `cargo test -p app info_panel_renders_metadata_features_and_resources`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app/src/main.rs
git commit -m "feat(picker): draw_info_panel content renderer"
```

---

### Task 9: Wire the panel into the picker loop (toggle, slide, lazy resolve)

**Files:**
- Modify: `crates/app/src/main.rs` (`run_story_picker` ~854; the draw closure; `draw_story_picker` area handling)

**Interfaces:**
- Consumes: `draw_info_panel` (Task 8); `picker::{resolve_aux, StoryAux}`; `anim::Tween`, `anim::parse` of `cfg.animation`; `saves_dir`, `hint_index` (already built in Task 7); `LIST_MIN_W`/`PANEL_MIN_W`.
- Produces: session-only panel state; `i`/`Tab` toggle with narrow-terminal guard; sliding width; lazy aux cache.

- [ ] **Step 1: Write the failing tests**

Add to `crates/app/src/main.rs` render tests. These exercise the pure slide-fraction helper and the area split (the interactive loop itself is not unit-tested; the geometry is):

```rust
#[test]
fn slide_fraction_interpolates_and_reverses() {
    // A closed→open slide at t=0 is 0.0, at t=1 is 1.0; reversing mid-slide
    // starts from the current fraction.
    let mut s = PanelSlide::closed();
    assert_eq!(s.fraction_at(0.0), 0.0);
    s.toggle_to(true, /*instant=*/true);
    assert_eq!(s.fraction_at(1.0), 1.0);
    s.toggle_to(false, true);
    assert_eq!(s.fraction_at(1.0), 0.0);
}

#[test]
fn panel_refuses_to_open_when_too_narrow() {
    // Below LIST_MIN_W + PANEL_MIN_W the toggle is a no-op.
    assert!(!can_open_panel(LIST_MIN_W + PANEL_MIN_W - 1));
    assert!(can_open_panel(LIST_MIN_W + PANEL_MIN_W));
}

#[test]
fn draw_story_picker_full_width_then_split() {
    use ratatui::{buffer::Buffer, layout::Rect};
    let cs = app::colors::ColorScheme::terminal_default();
    let sym = app::config::SymbolConfig::default();
    let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
    let stories = make_two_test_stories();
    let badges = vec![app::picker::RowBadges::default(); 2];
    let mut list = app::list_scroll::ListScroll::new();
    list.len(2);

    // Closed: list uses full width, no panel border cell on the right edge.
    let area = Rect::new(0, 0, 70, 12);
    let mut buf = Buffer::empty(area);
    let (list_area, panel_area) = split_picker_area(area, 0.0);
    assert_eq!(list_area.width, area.width);
    assert_eq!(panel_area.width, 0);

    // Open (fraction 1.0): list shrinks, a panel area with width >= PANEL_MIN_W appears.
    let (list_area, panel_area) = split_picker_area(area, 1.0);
    assert!(list_area.width < area.width);
    assert!(panel_area.width >= PANEL_MIN_W);
    let _ = (&stories, &badges, &glyphs, &cs, &mut buf, &mut list);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p app slide_fraction_interpolates_and_reverses panel_refuses_to_open_when_too_narrow draw_story_picker_full_width_then_split`
Expected: FAIL — `cannot find type 'PanelSlide'` / `split_picker_area`.

- [ ] **Step 3: Add layout constants + geometry helpers**

Near the top of the picker section in `crates/app/src/main.rs`:

```rust
const LIST_MIN_W: u16 = 24;
const PANEL_MIN_W: u16 = 28;

/// True if the terminal is wide enough to show list + panel.
fn can_open_panel(width: u16) -> bool {
    width >= LIST_MIN_W + PANEL_MIN_W
}

/// Split `area` into (list, panel) given an eased open fraction in [0,1].
/// Panel target width is a third of the area, clamped to
/// [PANEL_MIN_W, area.width - LIST_MIN_W]; the eased width is that × fraction.
fn split_picker_area(area: Rect, fraction: f64) -> (Rect, Rect) {
    if fraction <= 0.0 || !can_open_panel(area.width) {
        return (area, Rect::new(area.right(), area.y, 0, area.height));
    }
    let target = (area.width / 3).clamp(PANEL_MIN_W, area.width - LIST_MIN_W);
    let panel_w = ((target as f64) * fraction).round() as u16;
    let panel_w = panel_w.min(area.width - LIST_MIN_W);
    let list_w = area.width - panel_w;
    let list_area = Rect::new(area.x, area.y, list_w, area.height);
    let panel_area = Rect::new(area.x + list_w, area.y, panel_w, area.height);
    (list_area, panel_area)
}
```

- [ ] **Step 4: Add the `PanelSlide` state helper**

```rust
/// Session-only slide state for the info panel. Holds a target fraction and an
/// optional tween easing the displayed fraction toward it (so a mid-slide
/// reverse starts from the current position).
struct PanelSlide {
    open: bool,
    from: f64,
    to: f64,
    tween: Option<app::anim::Tween>,
}

impl PanelSlide {
    fn closed() -> Self {
        Self { open: false, from: 0.0, to: 0.0, tween: None }
    }

    /// The displayed fraction right now (tween-eased), given a raw progress.
    fn fraction_at(&self, progress: f64) -> f64 {
        app::anim::lerp(self.from, self.to, progress)
    }

    /// Current displayed fraction from the live tween (or the settled `to`).
    fn fraction(&self) -> f64 {
        match &self.tween {
            Some(t) => app::anim::lerp(self.from, self.to, t.progress()),
            None => self.to,
        }
    }

    fn active(&self) -> bool {
        self.tween.as_ref().is_some_and(|t| !t.done())
    }

    /// Toggle to `open`, arming a tween unless `instant`.
    fn toggle_to(&mut self, open: bool, instant: bool) {
        self.open = open;
        let target = if open { 1.0 } else { 0.0 };
        let current = self.fraction();
        self.from = current;
        self.to = target;
        self.tween = None; // set by caller with duration; see arm()
        if instant {
            self.from = target;
        }
    }

    /// Arm the tween with the configured duration/easing (call after toggle_to).
    fn arm(&mut self, cfg: &app::config::AnimationConfig) {
        if !cfg.enabled || cfg.scroll_ms == 0 || (self.from - self.to).abs() < f64::EPSILON {
            self.from = self.to;
            self.tween = None;
        } else {
            self.tween = Some(app::anim::Tween::new(
                std::time::Duration::from_millis(cfg.scroll_ms),
                cfg.easing,
            ));
        }
    }
}
```

(The Step-1 test calls `toggle_to(open, instant=true)` and reads `fraction_at`; keep those signatures. `arm` is used by the loop, not the unit test.)

- [ ] **Step 5: Add panel state + toggle handling in `run_story_picker`**

After building `row_badges`/`badge_glyphs`, add:

```rust
    let mut slide = PanelSlide::closed();
    let mut aux_cache: Vec<Option<app::picker::StoryAux>> = vec![None; stories.len()];
    let mut last_area = Rect::new(0, 0, 0, 0);
    let save_dir = saves_dir(&cfg.user_dir);
```

In the draw closure, capture the area and split it:

```rust
        let _ = terminal.draw(|f| {
            let area = f.area();
            last_area = area;
            let buf = f.buffer_mut();
            let (list_area, panel_area) = split_picker_area(area, slide.fraction());
            let (rects, vp) = draw_story_picker(
                &stories, &list, &row_badges, &badge_glyphs, dir, &cs, list_area, buf,
            );
            row_rects = rects;
            viewport = vp;
            if panel_area.width > 0 {
                if let Some(entry) = stories.get(list.selected) {
                    draw_info_panel(
                        &entry.title, &entry.filename, &entry.meta,
                        aux_cache[list.selected].as_ref(), panel_area, &cs, buf,
                    );
                }
            }
        });
```

Extend the tick condition so the loop keeps redrawing while the slide eases:

```rust
        if (list.has_active_animation() || slide.active())
            && !crossterm::event::poll(Duration::from_millis(16)).unwrap_or(false)
        {
            list.finalize_if_done();
            continue;
        }
```

Add the toggle keys to the key match (alongside the existing arms):

```rust
                    Char('i') | Tab => {
                        let target = !slide.open;
                        if !target || can_open_panel(last_area.width) {
                            let instant = !cfg.animation.enabled || cfg.animation.scroll_ms == 0;
                            slide.toggle_to(target, instant);
                            slide.arm(&cfg.animation);
                            // Resolve aux for the current row on first open.
                            if target {
                                ensure_aux(&mut aux_cache, &stories, list.selected, &save_dir, &hint_index);
                            }
                        }
                    }
```

`Tab` is `KeyCode::Tab` (already imported via `use crossterm::event::KeyCode::*`).

On selection-changing keys, resolve aux for the new row when the panel is open. After each movement arm (or once after the match), add:

```rust
        if slide.open {
            ensure_aux(&mut aux_cache, &stories, list.selected, &save_dir, &hint_index);
        }
        list.finalize_if_done();
```

- [ ] **Step 6: Add the `ensure_aux` helper**

```rust
fn ensure_aux(
    cache: &mut [Option<app::picker::StoryAux>],
    stories: &[app::picker::StoryEntry],
    idx: usize,
    save_dir: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) {
    if let Some(slot) = cache.get_mut(idx) {
        if slot.is_none() {
            if let Some(entry) = stories.get(idx) {
                *slot = Some(app::picker::resolve_aux(entry, save_dir, hint_index));
            }
        }
    }
}
```

- [ ] **Step 7: Update the footer/header hints**

Change the footer string in `draw_story_picker` to advertise the toggle:

```rust
    let footer = " ↑/↓ or j/k: move   PgUp/PgDn   Enter / click: open   i/Tab: info   q / Esc: quit";
```

And append an `[i: info]` marker to the header line:

```rust
    let header = format!(
        " lanthorn — choose a story  ({} found in {})   [i: info]",
        stories.len(),
        dir.display()
    );
```

- [ ] **Step 8: Run the geometry/slide tests + the full suite**

Run: `cargo test -p app slide_fraction_interpolates_and_reverses panel_refuses_to_open_when_too_narrow draw_story_picker_full_width_then_split`
Then: `cargo test -p app`
Expected: PASS across the app crate.

- [ ] **Step 9: Manual smoke check (documented, not automated)**

Run `cargo run -p app -- <a directory of stories>` and verify: rows show `Z`/`G` + artifact letters; `i` and `Tab` slide the panel in/out; the panel reflects the highlighted story; narrow terminals refuse to open the panel; `animation.enabled = false` toggles instantly. Note results in the commit body.

- [ ] **Step 10: Commit**

```bash
git add crates/app/src/main.rs
git commit -m "feat(picker): toggleable animated info side-panel (i/Tab)"
```

---

### Task 10: README + TODO completion

**Files:**
- Modify: `README.md` (document the picker info panel + row badges + `[symbols]` badge glyphs + `story_info`/`story_badge` theme selectors)
- Modify: `TODO.md` / `COMPLETED.md` via `scripts/todo-done`

**Interfaces:** none (docs only).

- [ ] **Step 1: Sync TODO/COMPLETED from main first**

Per project workflow, the worktree must not clobber `TODO.md`/`COMPLETED.md` edits made on main:

```bash
git fetch origin
git checkout origin/main -- TODO.md COMPLETED.md
```

- [ ] **Step 2: Document the feature in README.md**

Add a subsection under the picker/usage docs describing: the row badge cluster (type badge `Z`/`G` + `B`/`S`/`H` artifact letters, all configurable via `[symbols]` `badge_*`), the `i`/`Tab` info panel (metadata, features, resources, saves), the narrow-terminal fallback, and the `story_info` / `story_badge` style selectors. Match the existing README tone and depth.

- [ ] **Step 3: Complete the TODO item**

Run: `scripts/todo-done` for the "story list page … side-panel" item (line 7). This moves it to `COMPLETED.md` with a `TODO-xxxxxx` id and enforces the `Completes:` trailer.

- [ ] **Step 4: Commit with the required trailer**

```bash
git add README.md TODO.md COMPLETED.md
git commit -m "docs(readme): story-picker info panel + row badges

Completes: TODO-xxxxxx"
```

- [ ] **Step 5: Final full build + test**

Run: `cargo build -p app && cargo test -p app`
Expected: clean build, all tests pass.

---

## Self-Review

**Spec coverage:**
- Row badges (type + B/S/H, configurable glyphs, `story_badge` style) → Tasks 1, 5, 7. ✓
- Info panel (toggle `i`/`Tab`, slide, narrow guard, header/footer hints) → Task 9. ✓
- Panel content (title/fs/format/serial/ifid/features/resources/saves) → Task 8. ✓
- `StoryMeta` eager + header helpers → Task 4. ✓
- `RowBadges` eager all-rows → Tasks 5, 7. ✓
- `StoryAux` lazy + cache → Tasks 6, 9. ✓
- `read_archive_meta` cheap save meta → Task 3. ✓
- Themeable selectors → Task 2. ✓
- Config glyphs → Task 1. ✓
- README + TODO → Task 10. ✓
- Performance (no new scan reads except blorb-ext files; lazy aux; meta-only save reads; one saves readdir; one hint index) → Tasks 3, 4, 6, 7. ✓ (Documented deviation: self-blorb enumeration re-reads blorb-extension files, since extraction discards the resource index — bounded to blorb files only.)

**Placeholder scan:** The Task-8 `title_of`/`panel_filename` stubs are explicitly replaced in Step 3b's note by threading `title`/`filename` into `draw_info_panel`; Step 3c updates the test accordingly. No other placeholders.

**Type consistency:** `StoryMeta`/`ChunkInfo`/`Features`/`Engine`/`RowBadges`/`StoryAux`/`BadgeGlyphs` signatures match across Tasks 4–9. `resolve_aux(entry, save_dir, hint_index)`, `compute_row_badges(entry, save_names, hint_index)`, `draw_info_panel(title, filename, meta, aux, area, cs, buf)`, and `draw_story_picker(stories, list, badges, glyphs, dir, cs, area, buf)` are consistent between definition and call sites. `read_archive_meta(path) -> io::Result<Meta>` matches its `persist_files` consumer.

**Known correctness notes carried into tasks:**
- `load_hint_index(&cfg.user_dir)` (not `.join("hints")`).
- `read_archive_meta` preserves the `format_version` rejection.
- `resolve_sound_blorb` returns `(Blorb, PathBuf)`; only a *different* source path becomes `assoc_blorb`.
- `resolve_sound_blorb` only returns sibling/scan blorbs that *have sounds*; a graphics-only sibling blorb won't populate `assoc_blorb` (matches existing engine behavior; acceptable).
