# Live automapping

[← back to README](../../README.md)

- **Automatic room placement** as you explore — each new location is positioned
  relative to where you came from.
- **v4+ room detection** — for v4/v5 games that don't expose the room in the
  classic v3 status variable (Hitchhiker, Bureaucracy, A Mind Forever Voyaging),
  the room is read from the status line and resolved to a game object — preferring
  the player object's room when the game re-parents the player (Inform), falling
  back to a name-only room otherwise. Games that **center** the room name in their
  custom status display (Beyond Zork, Trinity) are handled too: the centered title
  is parsed and accepted only when it validates against the player's room, so those
  now automap as well. A hideable indicator in the map's
  bottom-right corner shows how the current room was found (`toggle-loc-method`,
  persisted via `show_loc_method`; styled by `loc_indicator`): `via player
  object`, `via name match`, `via name (unlinked)`, `via status variable`, or
  `via room heading`.
- **Glulx (Inform 7) room detection** — Glulx games automap too. The current
  room is read from the Inform room heading — the bold title line printed when
  you enter a room — so games that keep the room out of the status bar (e.g.
  FooFoo, Superluminal Vagrant Twin) still map. Rooms are matched by name, since
  the Glulx world model isn't introspectable; pre-game menus and character-setup
  screens correctly produce no room.
- **Nautical directions** — ship-based games (Seastalker and the like) that use
  *fore/aft/port/starboard* (plus *bow*/*stern*/*forward*) instead of the compass
  are understood: those movements map onto north/south/west/east so the vessel's
  decks lay out correctly.
- **Vertical connections laid out like N/S** — up/down moves place the new room
  directly north (up) / south (down) of its neighbor, shifting ordinary rooms aside
  just like a compass move, but yielding to confirmed reciprocal N/S adjacencies.
  They render as dotted connectors with up/down symbols (or a stairs glyph set),
  never as arrows, and never as "distorted" red edges. Up/down connections are
  routed through the same lane system as N/S compass connectors — crossing
  elimination and a border-centered anchor — and drawn as dotted lines with the
  up/down symbol on the room border. A matching Up+Down pair between two rooms
  collapses to a single dotted path with the symbol at both ends; if a reciprocal
  N/S connection also exists, it takes the center border slot and the up/down
  path yields to a fanned off-center slot. When a room pair is joined by both a
  compass direction and an up/down connection, only the compass path is drawn —
  the up/down connector is suppressed — but the room still shows its up/down
  symbol so vertical access stays visible. The map tidy also shifts rooms apart
  to keep up/down paths from crossing other paths where it can make room.
- **Connection routing** between rooms with overlap removal, so the map stays
  readable as it grows.
- **Layered maps** for multi-level areas, with manual layer controls.
- **Background tidy** — the layout re-optimizes itself as you discover rooms.
  Configurable: after every room (default), only on overlap, debounced every few
  rooms, or off (`background_tidy`).
- **Animated layout diagnostics** — step through the whole layout build stage by
  stage: a **Build** stop listing every connection, then **room-by-room placement**
  as each room drops onto the grid, then the relayout/overlap-cleanup passes — each
  move described ("moved 180 to clear overlap with 193") — to see and debug exactly
  how the map is assembled.
