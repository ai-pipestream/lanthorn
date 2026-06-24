# Verb/Item Menu (token palette) — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued. Depends on the inventory feature (L28) for inventory nouns. Touches `input.rs`/`main.rs`/`state.rs`/`keymap.rs` + a new render module.
**TODO item:** "Add menu based input selection for common verbs and items found in room description" (L15).

## Goal

A modal **token palette** for composing commands without typing: a two-pane menu of **verbs | nouns** (room items + inventory), plus a small **prepositions** group. Picking a token **appends it to the input line** (insert-for-editing), so multi-noun commands (`unlock door with key`) are built by picking tokens in sequence, then editing/submitting normally.

## Decisions

- **Two-pane layout:** verbs on the left, nouns on the right; a small prepositions group (third column or a row).
- **Nouns = room nouns + inventory items** (deduped). Room nouns come from the autocomplete data (`complete.rs` room words); inventory items from the inventory feature's `list_inventory` (see Dependencies).
- **Pick = insert token into the input line** (append with a single trailing space). This is how multi-noun commands are built — pick `unlock`, `door`, `with`, `key` → input becomes `unlock door with key`. The player then edits and presses Enter in the normal input line.
- **Multi-noun via prepositions:** a built-in preposition set (`with on in to under at from of`) is pickable, so verb+noun+prep+noun compositions are natural.

## Verbs & prepositions (curated, built-in)

The story dictionary does not distinguish verbs from nouns, so verbs are a **curated built-in list** (config-extendable later): `look examine take drop open close unlock lock push pull turn put give show read eat drink wear wuield(=wield) enter exit search move go north south east west up down in out inventory wait again`. (Trim/adjust to a sensible common set.) Prepositions: `with on in to under at from of`. A future `[verbs]`/`[prepositions]` config could extend these (out of scope for v1).

## UX

A modal opened by `Command::OpenVerbMenu` (keymap, default key + hotkey dialog group). `AppState.verb_menu: Option<VerbMenuState { pane: Pane, verb_idx, noun_idx, prep_idx }>` (`enum Pane { Verbs, Nouns, Preps }`). While open (a sub-mode in `key_to_action`, like saves/gallery):
- `Tab` / `←/→` switch pane; `↑/↓` move selection within the pane.
- `Enter` (or `Space`) **picks** the selected token → append it (+ space) to `state.input`.
- `Esc` / `q` close the menu (the composed text stays in the input line; the normal input-line `Enter` submits it).
The nouns pane is rebuilt from the current room nouns + inventory each time the menu opens.

## Components

- **`crates/app/src/render/verbmenu.rs` (new)** — `draw_verb_menu(state, area, buf)`: the two-pane (+prep) modal with the three lists and the active selection; an opaque background (use `Style::reset().bg(...)` per the panel-bleed lesson).
- **`state.rs`** — `verb_menu: Option<VerbMenuState>`; a helper to build the noun list (`room_nouns ∪ inventory`).
- **`input.rs`** — `Action::OpenVerbMenu`, `VerbMenuNav(...)`, `VerbMenuPick`, `VerbMenuClose`; the sub-mode router; `apply_action` appends the picked token to `state.input` (reusing the input buffer) and handles open/close/nav.
- **`keymap.rs`** — `Command::OpenVerbMenu` (+ default key, free; add to the hotkey "View" group).
- **`main.rs`** — render the modal when `verb_menu.is_some()`.
- **noun source** — reuse `complete.rs`'s room-word gathering (factor a `pub fn room_nouns(state) -> Vec<String>` if not already exposed) and `inventory::list_inventory` (player object from the inventory feature).

## Dependencies

- **Inventory (L28)** — for inventory nouns (`list_inventory` + the heuristic `player_obj`). Sequence L15 after L28 merges; if L28 is not yet present, fall back to room nouns only and note it.
- **Autocomplete (done)** — room nouns / `complete.rs`.

## Testing

- Noun list = dedup(room nouns ∪ inventory).
- `apply_action(VerbMenuPick)` appends the selected token + a space to `state.input` (e.g. picking `unlock` then `door` yields `unlock door `); building `unlock door with key` works token-by-token.
- Sub-mode keys: `Tab`/`←→` change `pane`; `↑↓` move the active index; `Esc` closes leaving `state.input` intact.
- Render test (TestBackend): the modal shows a known verb and a known room noun; opaque background (no bleed).
- The chosen toggle key → `Action::OpenVerbMenu`.

## Out of scope / non-goals

- Parsing the story for the real verb set (curated list instead).
- Auto-submitting (picks insert for editing; the player submits).
- Grammar validation of the composed command (the parser handles bad input as usual).
- Config-defined verb/prep lists (future).

## Risks & limitations (accepted)

- **Curated verbs may miss a game's custom verbs;** the player can still type. A future `[verbs]` config extends the list.
- **Noun freshness:** the noun list reflects the room at menu-open; reopening refreshes it.
- **Inventory dependency:** without L28's `list_inventory`, nouns are room-only until that feature lands.
