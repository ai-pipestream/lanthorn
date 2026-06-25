# In-Game Hint System — Design Spec

**Date:** 2026-06-24
**Status:** Approved (design via brainstorming Q&A) — pending user review of this doc.
**TODO item:** #47 ("Investigate a possible hint engine") — reframed to: help stuck players using standard IF hint files.
**Depends on:** the Z-machine VM (`GameSession`), the dialog chrome, the file-browser modal, the `.babelmap` archive/zip plumbing — all merged. No `mapper` changes; `zvm` changes only if a second concurrent VM session needs a tiny API (expected: none — `GameSession::new` + `submit`/`take_transcript` suffice).

## Goal

When a player is stuck, let them open a **Hints** panel that provides progressive, spoiler-safe hints from a standard IF hint file. babelmap is itself a Z-machine, so the first (and most native) source is the **Invisiclues `.z5`** companion file — it simply *runs* in a second session. The system is built around a **pluggable hint source** so a **UHS** reader can be added later as a second source. Adventures and hint files packaged in **zip** archives are supported.

## Research basis (why this design)
- **Invisiclues** (Infocom's progressive hints) are distributed digitally as **Z-code `.z5` files** (the PRIZM project) — designed to be run by a Z-machine interpreter. babelmap already has one.
- **UHS (Universal Hint System)** is the broader cross-game standard (tree of progressively-revealed, lightly-encrypted hints; open reference reader: OpenUHS). Bigger to parse; deferred to a later phase.
- Many games also embed their own `HINT` command — nothing to build there.

## Architecture

### Pluggable hint source
```rust
enum HintSource {
    /// A companion Invisiclues/hint program run as a second Z-machine session.
    Zcode(GameSession),
    // Future: Uhs(UhsDoc) — parsed UHS hint tree with our own navigation UI.
}
struct HintSession {
    source: HintSource,
    transcript: Vec<String>,   // the hint program's output (its own scrollback)
    scroll: u16,
    input: String,             // the hint panel's own input line
    label: String,             // e.g. "Invisiclues: Zork I"
}
```
`AppState.hints: Option<HintSession>` — `Some` while the panel is open. `any_overlay_open()` includes it.

### Presentation — modal mini-terminal
The Hints panel is a **centered modal** (dialog chrome, title from `HintSession.label`, `[X]` + Esc to close) whose content area is a mini-terminal for the active source:
- renders the hint session's `transcript` (word-wrapped, scrollable — reuse the transcript wrap helpers) + its own input line;
- **keystrokes route to the hint session** while open (typed lines `submit` to the hint VM; its output appends to the hint transcript), EXCEPT `Esc`/`[X]` (close) and the scroll keys;
- the **main game session is paused** (not advanced) while the panel is open — hinting never changes game state.

For the `Zcode` source this reuses the exact play-loop mechanics: `GameSession::submit(line)` + `take_transcript()`. The Invisiclues program drives its own menu (type a topic number, reveal one hint at a time) — we just host its terminal faithfully.

### Opening / discovery
`Command::OpenHints` → `Action::OpenHints` (a hotkey-dialog command + `/hints` slash command). On open, if `state.hints` is `None`, resolve a hint source via **discovery**, then start the session; if discovery finds nothing, fall back to the **file browser** to pick a hint file; remember the choice per story (keyed by IFID).

**Discovery order** (first hit wins), given the story path + its IFID:
1. **Remembered:** a per-story hint path saved from a prior manual pick (keyed by IFID, persisted in config/user-dir).
2. **Sibling files** next to the story: by naming convention — `<stem>.hints.z5`, `<stem>-hints.z5`, `<stem>.invisiclues.z5`, anything matching `*hint*`/`*clue*`/`*invisiclues*.{z3,z5,z8}` in the same dir, or such a file in a `hints/` subdir.
3. **Inside a zip:** if the story was loaded from a `.zip` (or a sibling `.zip` exists), look for a hint `.z*` entry inside it (same name patterns).
4. **Manual:** open the file browser to choose a hint file; on selection, remember it (step 1) and start the session.

### Zip support (adventures + hint files)
Loading is taught to accept `.zip` inputs:
- At story load (`main.rs` ~491), if the input file is a zip (magic `PK\x03\x04`), extract the first/only story `.z3/.z5/.z8` entry as the story bytes (the `zip` crate is already a dependency via `archive.rs`).
- Discovery (step 3) reads hint `.z*` entries from the same or a sibling zip.
- A small `load_story_bytes(path) -> io::Result<Vec<u8>>` helper centralizes "raw `.z*` file OR a `.z*` inside a zip"; `read_zip_entry(zip_path, predicate)` returns matching entry bytes.

## Phasing

- **Phase 1 — core hint panel (Zcode + modal + discovery sans zip):** `HintSession`/`HintSource::Zcode`, `Action::OpenHints` + `/hints` + hotkey command, the modal mini-terminal (render + input routing + close), discovery steps 1/2/4 (remembered + sibling files + file-browser), per-IFID remembered path. The main game pauses while open.
- **Phase 2 — zip support:** `load_story_bytes`/`read_zip_entry`; story loads from a zip; discovery step 3 (hint inside a zip).
- **Phase 3 — UHS reader (FUTURE, separate spec):** `HintSource::Uhs` + a UHS parser (hunks + decryption) + a hint-tree navigation UI in the same panel. Out of scope here; the `HintSource` enum + panel are the seam it plugs into.

## State / files
- `state.rs`: `hints: Option<HintSession>`; `HintSession`/`HintSource`; `any_overlay_open()` includes `hints`.
- `crates/app/src/hints.rs` (new): discovery (`resolve_hint_source`), `load_story_bytes`/`read_zip_entry`, the per-IFID remembered-path store.
- `crates/app/src/render/hints_panel.rs` (new): `draw_hints_panel(state, area, buf) -> Option<HintsРanelRects>` (dialog chrome + mini-terminal). Mirrors the other modal renderers.
- `main.rs`: `Action::OpenHints` handling, the hints-panel input intercept (route keys to the hint session), and zip-aware story load.
- `keymap.rs`: `Command::OpenHints` (kebab `open_hints`).
- `slash.rs`: `/hints` curated entry (caller-handled → open the panel).

## Testing
- Discovery: given a temp dir with a story + a `<stem>.hints.z5`, `resolve_hint_source` finds the sibling; with none, returns the "ask user" outcome; the name-pattern matcher unit-tested (positive + negative cases).
- Zip: `read_zip_entry` extracts a `.z5` from a built test zip; `load_story_bytes` returns identical bytes for a raw `.z5` and for that `.z5` packaged in a zip.
- Sub-session: opening hints on a tiny test hint `.z5` (reuse an existing fixture or a minimal one) yields a `HintSession` whose transcript contains the program's opening text; submitting a line advances it.
- Panel render (TestBackend): `draw_hints_panel` shows the title, the hint transcript text, the input line, and the `[X]`.
- `any_overlay_open()` true when `hints.is_some()`; main game state unchanged across open→submit→close.
- Slash `/hints` and `Action::OpenHints` open the panel (or trigger discovery).

## Out of scope / non-goals
- **UHS** parsing/rendering (Phase 3, separate spec).
- Auto-navigating the Invisiclues to the player's current room/puzzle (the invisiclues structure isn't machine-readable; we host its menu, the player picks).
- Bundling/shipping any hint files (the user supplies them; we discover/open them).
- Graphical UHS features (images/sounds) — text hints only when UHS lands.

## Risks & limitations (accepted)
- **Screen-model fidelity:** Invisiclues `.z5` are line-based menu programs, so the transcript model hosts them well; a hint file that heavily uses Z-machine screen features (windows/clear) may render imperfectly. Acceptable — these are rare for hint files; documented.
- **A second live VM** doubles VM memory while the panel is open; freed on close (drop the `HintSession`). Fine for the small hint programs.
- **Discovery false positives:** the `*hint*`/`*clue*` glob could match an unintended file; the file-browser fallback + remembered-path override cover it.

## Sources (hint-format research)
- Universal Hint System (format + open reader): https://github.com/Vhati/OpenUHS , https://en.wikipedia.org/wiki/Universal_Hint_System
- Infocom Invisiclues (PRIZM Z-code hints): https://ifarchive.org/indexes/if-archive/infocom/hints/invisiclues/
