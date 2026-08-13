# Maze representation: the matrix view

**Status:** implemented (SQ-0666). Settled design, iterated against real data
with the user, 2026-08-05; see `docs/features/mapping.md` for the shipped
behaviour and the fixture note in `unit_tests/README.md`. Example renders in this doc come from a
player's actual partial mapping of Colossal Cave's "all alike" maze
(`advent.blb`, save `default`, 12 rooms / 47 edges).

## The evidence

- **2 of 47 edges are reciprocal.** 18 return by a *different* direction,
  27 have no known return at all.
- **29 of 47 edges are marked distorted** — compass layout is wrong about
  ~62% of what it draws, because compass geometry is not what a maze is.
- **11 of 12 rooms are named "Maze"** — indistinguishable on screen.
- Every room records a `tried` direction list; a tried direction with no
  minted edge is usually a **self-loop**, which `Mapper::observe`
  structurally refuses (`location != prev_id`) — the classic "W leads back
  here" fact is currently thrown away.

Conclusion: inside a maze the player's knowledge is a **direction table per
room**. The map should show that table, not draw geometry that is 62% wrong.

## Core decision: the matrix is a view mode, not a maze feature

Every layer gets a **map view mode**: `drawn` (today's map) or `matrix`.
`/view-map` cycles it; the mode is per-layer and persists with the map.
A layer flagged `maze = true` (LayerMeta, persisted) merely *defaults* to
matrix. Tangle detection — a connected cluster whose non-reciprocal share
crosses a threshold — offers to peel the cluster to a new maze-flagged
layer; `/mark-maze-layer` flags an existing layer in place (the user's
hand-peeled "Maze" layer converts with one command). Detection is a
convenience that flips a default; nothing else depends on it.

> **Superseded (SQ-0826).** The asymmetry-based tangle detector described
> above shipped and was removed: it needs enough passages walked *both*
> ways before it can say anything, so it fired late, never, or on an
> ordinary ring of rooms. Flagging a maze is now a purely manual act
> (`/mark-maze-layer`). SQ-0439 revisits detection on evidence available at
> *first contact* instead. Everything else on this page still stands.

The matrix is also inherently screen-reader-friendly (a table linearises
where a drawn map cannot) — relevant to the SQ-0609 accessibility thread.

## The matrix view (settled form)

One row per room (first-visit order), one column per direction — **always
all twelve** (N S E W NE NW SE SW U D In Out): an untried cell in any
direction may be exactly what full exploration needs, so none are hidden.

```
               N     S     E     W    NE    NW    SE    SW     U     D     I     O
──────────────────────────────────────────────────────────────────────────────────
 Maze 1     →5⇠w    ⇢9    ⇢2    ⇢3     ·     ·     ·     ·     ·     ·     ·     ·
 Maze 2       ⇢3   ⇢10  →7⇠n    ⇢9     ·     ·     ·     ·     · →11⇠w     ·     ·
▸Maze 3    →11⇠u    ⇢5  →9⇠e →10⇠s     ·     ·     ·     ·    ⇢4     ·     ·     ·
 DeadEnd¹     ⇄4     ·     ×     ·     ·     ·     ·     ·     ·     ·     ·     ·
 Maze 4       ⇢1   ⇄DE  →5⇠s  →6⇠w     ·     ·     ·     ·    ⇢8    ⇢2     ·     ·
 ...
──────────────────────────────────────────────────────────────────────────────────
¹ Dead End, near Vending Machine    ⇱out: D from 11 → At West End of Long Hall
⇲ in:  At West End of Long Hall —S→ Maze 11
```

**Cell vocabulary** (one glyph + destination number, ≤6 cells wide):

| Cell        | Meaning |
|-------------|---------|
| `⇄4`        | reciprocal — the compass inverse returns |
| `→5⇠w`      | goes to 5; the return is via W (self-contained row) |
| `⇢9`        | one-way — no return known |
| `↩`         | self-loop — this direction leads back here |
| `⇱out`      | leaves the maze/layer; destination footnoted |
| `×`         | tried, no path found (a later observed loop upgrades it to `↩`) |
| `·`         | untried — the exploration frontier |

**Row furniture:** `▸` marks the room you are standing in; `⇲` marks a room
that a border edge from OUTSIDE the layer arrives at — the entrances the
`⇱out` cells have no mirror for, since an inbound edge's origin has no row
here to sit on. Precedence: `▸` wins when a room is both (you are standing
there; the entrance fact still reads in the footnote). Entrances are listed
one per line, `⇲ in:  <origin room> —<D>→ <target label>`, ordered by the
entering room's row position then by direction column, mirroring the
`⇱out` footnote's own convention. Rooms sharing a display name are numbered
by first-visit order ("Maze 1…11", display-only — identity is already
stable via object address); long or out-of-layer names are footnoted below
the table (`DeadEnd¹`, `⇱out` destinations).

**Selection cross-highlight:** selecting a row restyles — **bold via a
style selector, not a glyph** — every cell elsewhere that *arrives* at the
selected room: its known entrances, i.e. the answer to "how do I get back
here". Selection moves with ↑/↓ or by clicking a row; clicking a
destination cell jumps selection to that room's row.

**Geometry:** the full table is ~82 cells wide with return directions.
When the pane is too narrow for the full form, the matrix degrades before
it scrolls: first the `⇠x` return-direction suffixes drop (cells shrink to
`→5` / `⇢9` form, ~46 cells total — the return is still discoverable on
the destination's row and in its room-info card); only when even that
doesn't fit does horizontal scrolling engage (the label column stays
pinned), reusing the pane-scroll conventions. The threshold is computed,
not configured. Vertical scrolling as any list.

**Style selectors** (all new elements styleable per CLAUDE.md):
`map.matrix.header`, `map.matrix.row:here`, `map.matrix.row:selected`,
`map.matrix.cell:entrance` (the bold cross-highlight), `map.matrix.cell:frontier`
(`·`/`×` cells), `map.matrix.footnote`. Defaults reproduce the mockup
(bold for entrances, dim for frontier).

## Supporting changes (unchanged from the first draft)

1. **Self-loops become recordable** — lift the `observe` refusal into a
   first-class `origin == dest` edge (triple-keyed like Unknown edges),
   rendered as `↩` in the matrix and a badge on drawn-view room boxes.
   A `tried`-minus-edges heuristic may OFFER retroactive conversion but
   never assumes: only an observed same-room arrival proves a loop.
2. **Honest asymmetric edges in the drawn view** — arrowheads on one-way
   edges, both-end labels when directions disagree, selectors
   `map.edge:oneway` / `map.edge:asym`.
3. **Room card in the room-info panel** — the per-room exit table (the
   card form from the earlier draft) becomes the selected room's detail in
   room-info, complementing the matrix rather than competing with it.
4. **Walked-trail breadcrumb** — on maze-flagged layers, the last N
   traversed edges/rows highlight with a fading `map.trail` style (N ~ 8).

## Removals (user decision, 2026-08-05)

One representation per fact: with the matrix's `×`/`·` cells and the
room-info card carrying tried/untried per direction, the **room
inspector's explored rose** and the **"untried exits" listing** are
retired — they are older dialects for the same knowledge. The `tried`
data itself is untouched (it feeds the new cells); only the duplicate UI
surfaces go, along with their style selectors and template lines. If
implementation turns up further duplicate surfaces for exit-exploration
state, consolidate them into the card/matrix rather than keeping them.

## Persistence

LayerMeta: `maze: bool` and the per-layer view mode. Self-loop edges join
the connection list. Numbering derives from existing insertion order;
`tried` is already persisted. Formats may break freely, pre-release.

## Out of scope

- Force-directed layout for maze layers (dropped: with ~96% asymmetry a
  drawn graph communicates less than the table).
- Auto-solving/route hints through mazes.
- Item-drop auto-annotation (separate quest if wanted).

## Testing

Unit: cell classification (reciprocal/asym/one-way/self-loop/probe/untried)
against a fixture graph replicating the save's statistics; numbering
stability across save/load; entrance cross-highlight set; inbound border
edges (the `⇲` query, mirroring the `⇱out` `LeavesLayer` test from the
other side); footnote assignment; view-mode persistence per layer.
Integration: a sanitized copy of the advent.blb map.json in unit_tests/
(player data, redistributable): flag the layer, render the matrix, assert
known cells (`DeadEnd¹` row `E` is `×`, Maze 11 `D` is `⇱out`, the two
reciprocal pairs, ▸ on the current room, the fixture's one inbound edge —
"At West End of Long Hall" `S` into Maze 11 — produces the `⇲ in:` footnote
line and, because that room is ALSO the save's current room, ▸ wins over
⇲ on its row; bold entrances for a selected room in both honor_game_colours
modes). Falsification per CLAUDE.md throughout.
