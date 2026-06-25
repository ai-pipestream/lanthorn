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
`Command::OpenHints` → `Action::OpenHints` (a hotkey-dialog command + `/hint`/`/hints` slash command). On open, if `state.hints` is `None`, resolve a hint source via **discovery**, then start the session; if discovery finds nothing, fall back to the **file browser** to pick a hint file; remember the choice per story (keyed by IFID).

**Built-in `HINT` detection (suggested first).** Before/alongside external discovery, check whether the *story itself* supports an in-game hint command: babelmap already loads the story **dictionary** (for autocomplete), so test it for the words `hint`/`hints`. If present, surface a suggestion on the status line / at the top of the hints panel — "This game has its own hints — type `HINT` in the story." — since the game's built-in progressive hints are usually the best source. This is a non-blocking suggestion: the player can still open an external hint file. (Detection is a heuristic — a dictionary word `hint` strongly implies support but isn't a guarantee; phrased as a suggestion, never an auto-action.)

**Discovery order** (first hit wins), given the story path + its IFID:
1. **Remembered:** the per-IFID hint association from a prior pick/download (see *Persistence* below).
2. **Sibling files** next to the story: by naming convention — `<stem>.hints.z5`, `<stem>-hints.z5`, `<stem>.invisiclues.z5`, anything matching `*hint*`/`*clue*`/`*invisiclues*.{z3,z5,z8}` in the same dir, or such a file in a `hints/` subdir.
3. **Inside a zip:** if the story was loaded from a `.zip` (or a sibling `.zip` exists), look for a hint `.z*` entry inside it (same name patterns).
4. **Online match + offer to download:** if still nothing, look the story up in a known hint index by **IFID** (preferred), falling back to **title**; if a match is found, show a consent prompt ("Hints available for `<game>` — download?") and on accept **download + cache** the file under the user dir, then remember it (step 1) and open. NEVER auto-download — always prompt. (Phase 3 — see below.)
5. **Manual:** open the file browser to choose a hint file; on selection, remember it (step 1) and start the session.

### Persistence — what is saved/loaded
- **The live `HintSession` is transient.** It is NOT written into the `.babelmap` archive and does not survive closing the panel; reopening restarts the hint program at its top menu. (Hint programs re-navigate in seconds; persisting a second Quetzal blob per save is not worth the cost/complexity.)
- **The hint-file association IS persisted**, keyed by the story's **IFID**, in a small user-dir store (e.g. `~/.babelmap/hints/index.toml` mapping `ifid -> { path | cached_file }`). Written whenever a hint file is picked (manual) or downloaded; read at launch so discovery step 1 resolves instantly on subsequent runs. This per-IFID store — plus any **downloaded/cached hint files** under `~/.babelmap/hints/` — is the entirety of the "hint engine state" that is saved and loaded. It is independent of save slots (a hint belongs to a *story*, not a particular save).
- *(Deferred, low value:)* persisting the live invisiclues position (where in the menu the player was) — not done; reopening starts fresh.

### Zip support (adventures + hint files)
Loading is taught to accept `.zip` inputs:
- At story load (`main.rs` ~491), if the input file is a zip (magic `PK\x03\x04`), extract the first/only story `.z3/.z5/.z8` entry as the story bytes (the `zip` crate is already a dependency via `archive.rs`).
- Discovery (step 3) reads hint `.z*` entries from the same or a sibling zip.
- A small `load_story_bytes(path) -> io::Result<Vec<u8>>` helper centralizes "raw `.z*` file OR a `.z*` inside a zip"; `read_zip_entry(zip_path, predicate)` returns matching entry bytes.

## Phasing

- **Phase 1 — core hint panel:** `HintSession`/`HintSource::Zcode`, `Action::OpenHints` + `/hint`(`/hints`) + hotkey command, the modal mini-terminal (render + input routing + close), discovery steps 1/2/5 (remembered + sibling files + file-browser), built-in `HINT` detection (dictionary check → suggestion), per-IFID remembered-path store. The main game pauses while open.
- **Phase 2 — zip support:** `load_story_bytes`/`read_zip_entry`; story loads from a zip; discovery step 3 (hint inside a zip).
- **Phase 3 — online match + download:** discovery step 4 — match the story by IFID (then title) against a known hint index (Infocom Invisiclues on the IF Archive, via a bundled IFID/title → file table for the finite Infocom catalog), a consent prompt, download to `~/.babelmap/hints/`, cache + remember (per-IFID). Network + explicit consent; never auto-download.
- **Phase 4 — UHS reader (FUTURE, separate spec):** `HintSource::Uhs` + a UHS parser (hunks + decryption) + a hint-tree navigation UI in the same panel. Out of scope here; the `HintSource` enum + panel are the seam it plugs into. (UHS hint-file download could extend Phase 3's index later.)

## State / files
- `state.rs`: `hints: Option<HintSession>`; `HintSession`/`HintSource`; `any_overlay_open()` includes `hints`.
- `crates/app/src/hints.rs` (new): discovery (`resolve_hint_source`), `story_supports_hint(dictionary) -> bool` (the built-in `HINT` dictionary check), `load_story_bytes`/`read_zip_entry` (Phase 2), the per-IFID hint-index store (`load_hint_index`/`save_hint_assoc`, a small TOML under `~/.babelmap/hints/index.toml`), and (Phase 3) the online match-and-download (`match_hint(ifid, title)` against the bundled Infocom table + `download_hint(url, dest)`).
- `crates/app/src/render/hints_panel.rs` (new): `draw_hints_panel(state, area, buf) -> Option<HintsPanelRects>` (dialog chrome + mini-terminal; shows the built-in-HINT suggestion line when applicable). Mirrors the other modal renderers.
- `main.rs`: `Action::OpenHints` handling, the hints-panel input intercept (route keys to the hint session), zip-aware story load (Phase 2), and the download-consent prompt (Phase 3, reuse the dialog-confirm pattern).
- `keymap.rs`: `Command::OpenHints` (kebab `open_hints`).
- `slash.rs`: `/hint` and `/hints` curated entries (caller-handled → open the panel).

## Testing
- Discovery: given a temp dir with a story + a `<stem>.hints.z5`, `resolve_hint_source` finds the sibling; with none, returns the "ask user" outcome; the name-pattern matcher unit-tested (positive + negative cases).
- Built-in HINT: `story_supports_hint` returns true for a dictionary containing `hint`/`hints`, false otherwise.
- Persistence: `save_hint_assoc(ifid, path)` then `load_hint_index()` round-trips the mapping; an absent index loads empty; the live `HintSession` is NOT written into a `.babelmap` archive (assert an archive round-trip is unaffected by an open panel).
- Online match (Phase 3, no network): `match_hint(ifid, title)` returns the expected entry for a known Infocom IFID/title and `None` for an unknown story; the download-consent prompt is shown (not auto-fetched).
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
- **Built-in HINT detection is heuristic:** a `hint`/`hints` dictionary word strongly implies in-game hints but isn't a guarantee — surfaced only as a non-blocking suggestion, never an auto-action.
- **Online download (Phase 3):** network + explicit consent only; never auto-fetch. The bundled match table covers the finite Infocom catalog (the games with PRIZM Invisiclues); non-Infocom stories fall through to manual/UHS. Downloads are cached so it's a one-time fetch. Verify the source/license before bundling the table.

## Sources (hint-format research)
- Universal Hint System (format + open reader): https://github.com/Vhati/OpenUHS , https://en.wikipedia.org/wiki/Universal_Hint_System
- Infocom Invisiclues (PRIZM Z-code hints): https://ifarchive.org/indexes/if-archive/infocom/hints/invisiclues/
