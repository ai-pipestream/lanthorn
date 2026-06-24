# Hideable Inventory Panel — Design Spec

**Date:** 2026-06-23
**Status:** Approved (design) — queued (serial; touches `main.rs`/`state.rs`/`input.rs`/`transcript.rs`/`keymap.rs`).
**TODO item:** "Hide able inventory panel" (L28).

## Goal

A toggleable strip at the bottom of the story pane showing the player's carried items, sourced live from the Z-machine object tree when possible, falling back to parsing the game's own `inventory` output.

## Inventory sourcing (heuristic + parse fallback)

The Z-machine has no standard pointer to the **player object**, so we identify it heuristically and fall back to parsing.

### Heuristic player-object detection
The player object is the one whose parent **follows the current room** across moves (consistent with how we map via global 0).
- Each turn compute `current_location` (zvm `current_location`, global 0) and `objects_here = { O : zvm::objects::get_parent(mem, O) == current_location }` (scan `1..=max_object_number`).
- Track `prev_location` and `prev_objects_here` in `AppState`.
- On a **location change** A→B (both nonzero, A≠B): the player is an object in `objects_here(B)` that was in `prev_objects_here` (it came from A to B). If exactly one such object, **lock** `player_obj = Some(O)`. (If zero/many, leave unlocked and try again next move.)
- Once locked, `player_obj`'s parent tracks the room automatically; inventory = its children.

### Live inventory
When `player_obj` is locked: walk `get_child(player_obj)` then the `get_sibling` chain, `short_name` each → the carried items (top-level only for v1; note nested containers as a future extension). This is always current.

### Parse fallback
When `player_obj` is NOT yet locked, use the last parsed `inventory` output:
- Detect when the submitted command is an inventory command (`i`, `inv`, `inventory`, case-insensitive, trimmed).
- Capture that turn's output and parse it: after a header line matching `carrying|holding|have|You are empty` (case-insensitive), take the subsequent non-empty listed lines (strip leading `a/an/the` and bullets/whitespace) as items; an "empty-handed" phrase → empty list. Store in `AppState.inventory_fallback: Vec<String>`.
- The panel shows live children when locked, else the fallback list (with a subtle "press i" hint when neither is available).

A small pure module **`crates/app/src/inventory.rs`** holds the testable logic: `detect_player_obj(...)`, `list_inventory(mem, player_obj) -> Vec<String>`, `parse_inventory_output(text) -> Vec<String>`.

## Panel (strip in the story pane)

- Rendered in `crates/app/src/render/transcript.rs` (it already lays out status line / transcript / suggestion line / input line). When `state.show_inventory`, draw a 1–2 row **inventory strip just above the input line**: `Inv: lamp, sword, leaflet` (truncated to width; "Inv: (empty)" / "Inv: (press i)"). The transcript area shrinks by the strip height (same pattern as the autocomplete suggestion line).
- Toggle: `Command::ToggleInventory` → `Action::ToggleInventory` flips `AppState.show_inventory: bool` (default false). Added to the keymap (default in the hotkey dialog's "View" group; the user can promote it to `direct`). Pick a default key (e.g. `v`); verify it is free.

## State additions (`state.rs`)

```rust
pub show_inventory: bool,                 // default false
pub player_obj: Option<u16>,              // locked player object, None until detected
pub inventory_fallback: Vec<String>,      // last parsed `i` output
// tracking for detection:
pub prev_location: Option<u16>,
pub prev_objects_here: std::collections::BTreeSet<u16>,
```

## Wiring (`main.rs` event loop)

After `apply_turn` each turn:
1. Update player-object tracking: compute `current_location` + `objects_here`; run `inventory::detect_player_obj(prev_location, &prev_objects_here, current_location, &objects_here)` → if it returns `Some(O)`, set `state.player_obj`; then store `prev_location`/`prev_objects_here`.
2. If the just-submitted command was an inventory command, `state.inventory_fallback = inventory::parse_inventory_output(&result.transcript)`.
The panel render reads `state.player_obj` (→ live `list_inventory`) or `inventory_fallback`.

## Footprint

`crates/app/src/inventory.rs` (new), `state.rs` (fields), `render/transcript.rs` (the strip), `input.rs` + `keymap.rs` (`ToggleInventory` command/action + default binding), `main.rs` (per-turn tracking + capture). zvm is read-only (`get_parent`/`get_child`/`get_sibling`/`short_name`/`current_location` all exist). Do NOT modify `mapper` or `zvm`.

## Testing

- `detect_player_obj`: given prev_location=A, prev_objects_here={player, troll}, current=B, objects_here={player, sword} → returns `Some(player)` (the object in both moved-from and moved-to); ambiguous/zero cases → None.
- `parse_inventory_output`: "You are carrying:\n  a brass lamp\n  a sword" → `["brass lamp","sword"]`; "You are empty-handed." → `[]`; non-inventory text → `[]`.
- `list_inventory` (fixture-backed, minizork present): lock a known player object and assert it lists known starting items, OR unit-test the child-walk on a synthetic memory.
- Strip render (TestBackend): with `show_inventory` and a non-empty list, the strip shows `Inv:` + an item; hidden when false; transcript shrinks by the strip height.
- Key: the chosen toggle key → `Action::ToggleInventory`; `apply_action` flips the bool.

## Out of scope / non-goals

- Nested container contents (show only top-level carried items in v1).
- Interacting with items from the panel (it is read-only display).
- Worn/equipped distinction, weights, or item descriptions.
- `mapper` / `zvm` changes.

## Risks & limitations (accepted)

- **Heuristic mis-detection:** in unusual games the player object may not lock cleanly (multiple objects moving together, or the player never changes rooms); the parse fallback covers these, and the strip degrades to "(press i)".
- **Parse fragility:** inventory output wording varies; the parser is best-effort and game-specific phrasing may slip through — acceptable since the live heuristic is the primary path once the player moves once.
- **Object scan cost:** `objects_here` scans all objects each turn; object counts are modest and this is once per turn — negligible.
