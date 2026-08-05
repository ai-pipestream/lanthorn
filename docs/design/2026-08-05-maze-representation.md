# Maze representation: making tangles legible

**Status:** approved direction, grounded in real data; not yet implemented.

## The evidence

From a player's partial mapping of Colossal Cave's "all alike" maze
(`advent.blb`, save `default`, rooms hand-peeled onto a "Maze" layer —
deliberately an *incomplete* maze, which the design must serve):

- 12 rooms, 47 internal edges, 2 border edges.
- **2 of 47 edges are reciprocal.** 18 return by a *different* direction,
  27 have no known return at all.
- **29 of 47 edges are marked distorted** — the compass layout is wrong about
  ~62% of what it draws, because compass geometry is not what a maze is.
- **11 of 12 rooms are named "Maze"** — indistinguishable on screen.
- Every room carries a `tried` direction list; comparing it to recorded edges
  exposes information the map cannot show: a direction that was tried and
  minted no edge is (almost always) a **self-loop**, which
  `Mapper::observe` structurally refuses to record (`location != prev_id`
  guard) — so the classic "W leads back here" maze fact is thrown away.

Conclusion: inside a maze, drawn compass edges are the wrong medium. The
knowledge a maze player accumulates is a **direction table per room**, plus
asymmetry, self-loops, and which directions remain untried.

## Components

### 1. Maze layers, detected and manual

The layer/peel machinery is the container. Add tangle detection: a connected
room cluster whose non-reciprocal-edge share crosses a threshold (the data
says even 50% is generous; real mazes sit near 95%) triggers a one-time
suggestion to move the cluster to a new layer flagged `maze = true`
(LayerMeta gains the flag; persisted). Manual: `/mark-maze-layer` toggles the
flag on the active layer, so an existing hand-peeled layer (like this save's)
converts in place. The main map shows a maze layer's rooms as today (layers
already do that); the flag changes how the layer *renders when active*.

### 2. Tangle view: exit tables, not edges

When the active layer is flagged maze, the map pane renders **room cards in a
stable grid** (sorted by first-visit order, which numbers them naturally) and
draws no geometric edges at all:

```
┌ Maze #1 ──────────┐  ┌ Maze #2 ──────────┐  ┌ Maze #3 (here) ───┐
│ N → #2            │  │ N → #2 ↩          │  │ N → #1            │
│ E → #5            │  │ S → Dead End      │  │ E → ?             │
│ W → #1 ↩          │  │ W → #4   ⇠ none   │  │ SW · untried      │
│ S ⇢ #4 (one-way)  │  │ U → Pit ⇱ border  │  └───────────────────┘
└───────────────────┘  └───────────────────┘
```

Card rows, per direction: destination (numbered), `↩` self-loop, `⇢`
one-way (no known return), `⇠ none` (asymmetric — the return comes from a
different direction, shown on the destination's card), `?` tried-but-unknown,
`· untried` (from `tried` complement against the room's known exits), and
border exits out of the maze. The selected card's destinations highlight in
place — navigation is card-to-card, not spatial. Clicking a destination
jumps selection to that card.

This is the partial-maze answer: `?` and `· untried` rows make the frontier
explicit, so an in-progress maze reads as a checklist of what remains.

### 3. Self-loops become recordable

Lift the `location != prev_id` refusal into a first-class self-loop edge
(`origin == dest`, keyed like Unknown edges by the full triple). Rendering:
never a drawn loop — a `↩` marker on the direction row (tangle view) or a
compact badge on the room box (`↩N,W`) on normal layers. Retroactively, the
`tried`-minus-edges heuristic can OFFER conversion ("3 tried directions led
nowhere new — record as self-loops?") but must not assume: a failed/refused
move also leaves a tried entry, and only a same-room arrival proves a loop,
so the automatic path records only genuinely observed self-arrivals from now
on.

### 4. Honest asymmetric edges on normal layers

Outside tangle view (and for the few mazes not worth a layer): arrowheads on
one-way edges; when a pair's directions disagree, label both ends
(`E→ … →SW`); the existing distorted styling stays. New selectors:
`map.edge:oneway`, `map.edge:asym` (+ glyph config), defaults matching the
current edge style so nothing changes visually until styled.

### 5. Same-name numbering and notes

Within a layer, rooms sharing a display name get stable ordinal suffixes
("Maze #3") derived from first-visit order — display-only, identity is
untouched (Glulx object address via room-lock already keeps them distinct).
The existing per-room notes field carries the classic dropped-item
annotations and shows on the card's second line when set.

### 6. Walked-trail breadcrumb

While the active layer is a maze layer, highlight the last N traversed
edges/cards (N ~ 8, config) with a fading trail style (`map.trail`
selector). Orientation while mapping is half the maze problem.

## Persistence

LayerMeta gains `maze: bool`; self-loop edges join the connection list
(format may break old files freely, pre-release). Card numbering derives
from room insertion order already present in the save; nothing new persists
for it. `tried` is already persisted per room.

## Out of scope

- Force-directed geometric layout for maze layers — considered and dropped:
  with 96% asymmetry the resulting hairball communicates less than cards do.
- Auto-solving/pathfinding through mazes (route hints).
- Item-drop detection (auto-annotating cards from inventory changes) —
  attractive, separate quest if wanted.

## Testing

Unit: tangle detection thresholds on synthetic graphs; card row derivation
(self-loop/one-way/asym/untried classification) against a fixture graph built
to this save's exact statistics; numbering stability across save/load.
Integration: load a copy of the advent.blb map.json fixture (sanitized into
unit_tests/ — it is player data, not game data, so redistributable), flag the
layer, assert the tangle view rows for known rooms (#864's E → `?`/untried,
the Dead End border edge, the 2 reciprocal pairs). Falsification per
CLAUDE.md throughout.
