# Cover Art Preview — Design Spec

**Date:** 2026-07-02
**Status:** Approved for planning
**Feature:** Display a game's Blorb frontispiece (cover art) in the story-picker info panel.

## Goal

When browsing games in the story picker, show the selected game's cover art
(the Blorb `Fspc` frontispiece image) in the right-hand info panel, rendered
with the best terminal graphics protocol available and a universal half-block
fallback.

## Scope

**In scope**
- Parse the Blorb `Fspc` (frontispiece) chunk to learn the cover Pict resource number.
- Decode that Pict (PNG/JPEG) and render it in the picker info panel.
- Auto-detect the best terminal image protocol (Kitty / iTerm2 / Sixel), with a
  Unicode half-block fallback that works on every terminal.
- A CLI flag to force the render mode (for testing the fallback on capable terminals).

**Out of scope (explicitly deferred)**
- In-game graphics of any kind (Glulx Glk graphics windows / inline images).
- Z-machine V6 graphics.
- Any image source other than the designated frontispiece (no heuristic
  "first Pict" fallback).
- A config-file toggle or F-key toggle for the cover (the panel already toggles
  with `i`/Tab; YAGNI until asked).

## Decisions (locked)

| Question | Decision |
|---|---|
| Where shown | Story-picker info panel (right pane). No in-game layout changes. |
| Cover source | Blorb `Fspc` frontispiece **only**. No `Fspc` → no cover, panel unchanged. |
| Render mode | Auto-detect best protocol per terminal; half-block fallback. |
| Test override | CLI flag `--image-protocol <auto\|halfblocks\|kitty\|iterm2\|sixel>`, default `auto`. |
| Data flow | Lazy: decode only the currently-selected game, on selection change, cached. |
| Failure behavior | **Silent** — any failure (no `Fspc`, missing resource, bad/unsupported image, panel too small) shows no cover; the panel behaves exactly as it does today. No placeholder text. |
| Cover size | Top of the info panel, full panel width (minus border), aspect-preserved, capped at 50% of panel height. Text flows below. |

## Global Constraints

- **`blorb` crate stays zero-dependency.** `Fspc` parsing is pure byte-reading.
- **VM crates (`zvm`, `gvm`) stay zero-dependency and are untouched.**
- New third-party dependency (`ratatui-image`, and its `image` decoder) is added
  to the **`app` crate only**. `image` features trimmed to png + jpeg.
- **Cross-platform:** must run on Windows/Linux/macOS. Satisfied by the half-block
  fallback (real-pixel protocols are a progressive enhancement).
- **Themeable UI:** the one new themeable element (the cover-region letterbox
  fill) gets a `picker.cover` style selector (ColorScheme field + `style.rs`
  selector + render apply), per the project's styling rule.
- Targets ratatui 0.30 / crossterm 0.29 (already on the branch history at `598b6d0`).
- README updated on completion (cover art is a major feature).

## Architecture

Four isolated units, each with one responsibility and a narrow interface.

### Unit 1 — `blorb`: frontispiece parsing (zero-dep)

**What it does:** exposes which Pict resource is the cover.

**Change:** `Blorb::parse` currently stops walking top-level chunks the moment it
finds `RIdx` (`crates/blorb/src/lib.rs:99-103`). Extend the walk to continue past
`RIdx` and capture the top-level `Fspc` chunk — a single 4-byte big-endian Pict
resource number — storing it on the `Blorb` struct.

**Interface added:**
```rust
/// The frontispiece (cover) Pict resource number, if the blorb declares one.
pub fn frontispiece(&self) -> Option<u32>;
```
Consumers combine it with the existing
`resource(b"Pict", n) -> Option<(&[u8;4], &[u8])>` to get `(chunk_type, bytes)`.

**Depends on:** nothing new (std only).

### Unit 2 — `app::cover`: decode + protocol builder (new module)

**What it does:** turns raw Pict bytes into a renderable ratatui-image protocol,
and caches the result.

**Interface (sketch):**
```rust
pub struct CoverCache { /* Option<(PathBuf, Protocol)> */ }

impl CoverCache {
    /// Build (or reuse cached) a cover protocol for `key`'s image bytes.
    /// Returns None on any decode failure or when bytes are None.
    fn ensure(&mut self, picker: &Picker, key: &Path, bytes: Option<&[u8]>) -> Option<&Protocol>;
}
```
- Decodes via the `image` crate; builds the protocol via a shared
  `ratatui_image::picker::Picker`.
- Cache key is the selected game's path; rebuilt only when the key changes.
- Knows nothing about picker UI geometry — pure bytes→renderable.

**Depends on:** `ratatui-image`, `image`.

### Unit 3 — `app` picker glue (`main.rs`, minimal)

**What it does:** wires selection → bytes → cover, and mounts the widget.

- Initialize one `Picker` at picker startup: `Picker::from_query_stdio()` for
  `auto`, or a fixed `ProtocolType` when the CLI flag forces a mode. Done once,
  after the terminal is in raw/alt-screen mode.
- On selection change, inside the existing `ensure_aux` lazy path
  (`crates/app/src/main.rs:942`, `picker.rs:146-170`): re-read `StoryEntry.path`
  → `Blorb::parse` → `frontispiece()` → `resource(b"Pict", n)` → hand bytes to
  `CoverCache::ensure`.
- In `draw_info_panel` (`crates/app/src/main.rs:1258`): carve a cover region from
  the top of `inner` (`frame.content`), render the cached `Image` widget there,
  offset `inner.y` / reduce `content_height` before the existing text loop so the
  scroll math (`main.rs:1372-1373`) stays correct. Fill uncovered cells with the
  `picker.cover` style. Skip entirely when the panel is closed or narrower than
  the existing `can_open_panel` / `PANEL_MIN_W` threshold (`main.rs:857-862`).

**Depends on:** Units 1, 2.

### Unit 4 — CLI flag

`--image-protocol <auto|halfblocks|kitty|iterm2|sixel>` on the app binary
(clap, alongside existing flags), default `auto`. Maps to ratatui-image's
`ProtocolType` (or the query path for `auto`). Threaded into the picker startup
that builds the `Picker`.

## Data flow

```
selection changes
  → ensure_aux(path)
      → fs::read(path) → Blorb::parse
      → frontispiece()  ── None ─────────────→ cover = None (panel unchanged)
      → resource(b"Pict", n)  ── None ───────→ cover = None
      → CoverCache::ensure(picker, path, bytes)
            ── decode Err ──────────────────→ cover = None
            ── Ok ──────────────────────────→ cached Protocol (key = path)
draw_info_panel(frame)
  → cover present?
       yes → reserve top rows (≤50% panel height, aspect-scaled),
             render Image widget, letterbox fill = picker.cover style,
             text lines flow below
       no  → panel exactly as today
```

## Error handling

Every failure path resolves to "no cover, panel unchanged," silently:
- No `Fspc` chunk / `frontispiece()` is `None`.
- `Fspc` names a Pict number with no matching resource.
- Unsupported or corrupt image bytes (decode returns `Err`).
- Panel closed or too narrow.

Decode is wrapped so a malformed image can never panic the picker event loop.
No log spam, no placeholder text.

## Testing

- **blorb (`crates/blorb/src/lib.rs` tests):** hand-built Blorb bytes with an
  `Fspc` chunk → `frontispiece()` returns the encoded number; absent `Fspc` →
  `None`; `Fspc` present but referenced Pict absent → `frontispiece()` returns the
  number and `resource(b"Pict", n)` returns `None` (caller degrades to no cover).
- **cover decode (`app::cover` tests):** a synthetic in-memory PNG → decodes to
  the expected dimensions; garbage bytes → `Err`, handled without panic; cache
  reuses on identical key and rebuilds on key change.
- **render (`app` test, `TestBackend`):** force `--image-protocol halfblocks`,
  render the info panel with a cover, assert the reserved top region has
  non-blank cells and the text lines lay out below the reserved region.
  (Protocol modes emit terminal escape sequences that are not deterministically
  snapshot-testable; the half-block path is the testable one — a core reason the
  force flag exists.)

## Verification

- Workspace builds; full test suite passes (baseline 1769 + new tests).
- `cargo clippy --workspace --all-targets` clean (0 warnings — project standard).
- Manual: a blorbed game with a known cover shows the image in the info panel on
  a protocol-capable terminal, and half-blocks with `--image-protocol halfblocks`.
- `blorb` and VM crates remain zero-dependency (empty `[dependencies]` unchanged
  for VM crates; `blorb` gains nothing).
