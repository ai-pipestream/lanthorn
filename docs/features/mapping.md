# Live automapping

[← back to README](../../README.md)

Play the game; the map draws itself. Every room you enter and every exit you take
is boxed, connected, and de-overlapped on the fly, then continuously nudged into a
clean layout — no graph paper, no pausing to annotate, no manual placement. Walk
north and a new room slides into place north of where you stood; double back and
the connection closes into a loop. This is babelmap's flagship feature, and it is
the reason the map pane earns half your terminal.

The mapper is deliberately **engine-agnostic**. It never sees a Z-machine opcode
or a Glk call — it consumes a plain stream of *locations* and *movements* and
turns it into a spatial graph. That means the **same automapper draws every
game**, whether you're charting the Great Underground Empire in *Zork*, threading
*Counterfeit Monkey* in Glulx, or exploring *Adventureland* in a classic Scott
Adams adventure. One map builder, three engines, zero special cases.

![babelmap playing Zork I with a live automap of the Great Underground Empire](../automapping.png)

## Knowing where you are — across three engines

Before the mapper can place a room it has to be told which room you're in, and
each engine surfaces that differently. babelmap handles all of it, and records
*how* it worked out each room the first time it finds it — click a room to see
"Found by:" in the room inspector. It is kept with the room, so the answer is
still there long after the turn that discovered it.

- **Classic Z-machine (v3)** reports the room in the status-line variable —
  `via status variable`.
- **v4/v5 Z-machine games that hide it** (Hitchhiker, Bureaucracy, A Mind
  Forever Voyaging) don't expose a room in the classic variable, so the room name
  is read off the status line and resolved back to a game object — preferring the
  player object's room when the game re-parents the player, Inform-style
  (`via player object`), and falling back to a name-only room otherwise
  (`via name match`, or `via name (unlinked)` when it can't be tied to an object).
  Games that **center** their room title in a custom status display (Beyond Zork,
  Trinity) are parsed too — the centered heading is accepted only once it
  validates against the player's room — so those now automap as well.
- **Glulx (Inform 7)** games often keep the room out of the status bar entirely,
  so babelmap reads the **Inform room heading** — the bold title line printed as
  you enter a room (`via room heading`). Games like FooFoo and Superluminal
  Vagrant Twin map cleanly this way; rooms are matched by name since the Glulx
  world model isn't introspectable, and pre-game menus or character-setup screens
  correctly produce no room.
- **Scott Adams** adventures feed their locations straight through the same
  engine-agnostic pipeline — nothing special to configure.

## Getting around the map

The map is a place you can move through, not just a picture.

- **Zoom** — `zoom-map in|out|reset` (or a signed step) scales between a detailed
  boxed view and a compact overview.
- **Pan** — `pan-map <dx> <dy>` slides the viewport; `center-map` snaps back to the
  selected room, or the room you're standing in.
- **Layer tabs** — multi-level areas are split into named **layers** shown as a tab
  strip across the top of the map (e.g. `Main  Cellar  Maze`, each with its room
  count); the active tab is highlighted. `cycle-layer next|prev` switches between
  them. Carve a region off with `peel-layer` or fold one back with `merge-layer`.
  A bare `peel-layer` cuts at the passage you just walked in through — step into the
  maze, peel, and the maze goes to its own layer — which works even when the entrance
  is one-way or the way back is some other direction entirely. `peel-layer <direction>`
  names the seam yourself; with neither, it falls back to hunting for a stairway or
  other portal to cut at.
- **Room inspector** — `toggle-inspector` opens an overlay for the selected room:
  its id, name, layer, position, and the per-edge layout constraints, so you can
  see *why* a room landed where it did.
- **Hand edits** — select rooms with `select-room next|prev`, `nudge-room` a stray
  box into place, `rename-room` / `rename-layer`, jot `edit-notes`, or clean up the
  graph with `delete-connection` and `relabel-edge`. Room-number labels toggle with
  `toggle-room-numbers`.
- **Export** — take the map with you: `export-svg` writes a scalable vector image,
  `export-dot` emits Graphviz DOT (render it with `dot -Tsvg …`), and `export-map`
  writes the raw structure. Omit the filename for a default path in the game's data
  directory. A saved map can be reopened later with `load-map`.

## Connections that stay readable

A naïve "one arrow per exit" map dissolves into spaghetti fast. babelmap routes
connections through a lane system with crossing-elimination and overlap removal,
and it understands the awkward cases:

- **Vertical connections** — up/down moves place the new room directly north (up)
  or south (down) of its neighbour, shoving ordinary rooms aside like a compass
  move but yielding to confirmed reciprocal N/S adjacencies. They render as dotted
  connectors with up/down (or stairs) glyphs — never as arrows, never as "distorted"
  red edges. A matching Up+Down pair between two rooms collapses to a single dotted
  path marked at both ends. Where a room pair is joined by *both* a compass direction
  and a staircase, only one line is drawn — see below for which wins.
- **Nautical directions** — ship games (Seastalker and kin) that steer by
  *fore / aft / port / starboard* (plus *bow* / *stern* / *forward*) instead of the
  compass are understood: those map onto north / south / west / east so the vessel's
  decks lay out correctly.
- **Combined multi-direction paths** — two rooms get **one** line between them, however
  many ways you can actually walk it. Zork's around-the-house ring links each pair by
  both a cardinal and a diagonal; Adventure's maze will happily connect the same two
  rooms four different ways, and a staircase often shadows a compass passage. Drawing
  them all means lines that exist only to cross each other, so babelmap picks a single
  representative: a **reciprocal** pairing first — the two ends are exact opposites, so
  the line runs straight and each arrowhead points the way you really travel — and
  otherwise by direction priority, **N, S, E, W, NE, NW, SE, SW, up, down**. The line
  that wins keeps its own arrowhead (or `↑`/`↓` if a staircase won), and the passages
  that lost draw nothing at all. Lines carrying more than one passage are tinted with
  the `shared_path` selector so you can see there is more to the story — and the room
  inspector lists every exit with its direction and destination, so nothing is lost,
  only unstacked.

Confirmed reciprocal N/S and E/W adjacencies are treated as inviolable: an up/down
move yields rather than shove a reciprocal partner off its shared column or row, and
overlap cleanup may only slide a reciprocal room *along* its own axis, never off it.

## Keeping the layout tidy

The whole map re-optimizes itself as you discover rooms, so it stays readable as it
grows. How eagerly is up to you (`background_tidy`): after every new room (the
default), only when a new room overlaps an old one (`on_overlap`), debounced every
few rooms (`debounced`), or off entirely. Force a pass any time with `tidy-map`.

Curious how a layout got built? `animate-tidy` steps through the whole assembly
stage by stage — a **Build** stop that lists every connection, then
**room-by-room placement** as each box drops onto the grid, then the
relayout/overlap-cleanup passes with each move described ("moved 180 to clear
overlap with 193"). Step it with `anim-step forward|back`, play/pause with
`anim-play`, and leave with `anim-exit`. It's equal parts diagnostic and quietly
mesmerising.

## Making it yours

Every glyph the map draws is a themeable preset in the `[symbols]` section of
`style.toml` — swap the whole look without touching a line of code:

- `box_style` — room outlines: `rounded` (default), `thick`, `double`, `solid`,
  `super-thick`, `ascii`, or `borderless`.
- `arrow_set` — connector arrowheads: `filled` (default), `line`, or a family of
  Nerd Font sets (`nerdfont`, `nf-bold`, `nf-box`, `nf-circle`, `nf-outline`) for
  patched fonts.
- `path_style` — the line-art that draws the connectors: `light` (default),
  `heavy`, or `dotted`.
- `portal_icons` — up/down/in/out markers: `ascii` (default), `nerdfont`, or
  `nerdfont-stairs` for distinct stairway icons.

Individual glyphs can be overridden one at a time in `[symbols.overrides]`, and
`diagonal_corners` toggles the half-diagonal corner stubs off for fonts without
Unicode 13 coverage. Reload changes live with `reload-style`. See
[customization & configuration](customization.md) for the full styling surface, and
[interface](interface.md) for mouse-driven map navigation.
