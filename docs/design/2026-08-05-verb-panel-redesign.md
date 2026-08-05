# Verb panel redesign: the command band

**Status:** approved design, not yet implemented. Supersedes
`docs/superpowers/specs/2026-06-23-verb-menu-design.md` (the shipped left-dock
token palette) — that spec's "out of scope" list deferred everything that made
the panel usable.

## Why the current panel fails

The shipped verb menu (`crates/app/src/render/verbmenu.rs`) is a left-edge
slide-in dock with three stacked lists — 35 hardcoded verbs, 8 prepositions,
and a noun list — reached only through the hotkey leader (`ctrl+p v`). Its
defects are structural, not cosmetic:

- **The noun list is scraped, not known.** `build_verb_menu_nouns`
  (`input.rs`) tokenizes the last 20 transcript lines and adds a stale
  inventory snapshot that only exists if the player typed `i` recently. The
  engine's live object tree (`Engine::introspect()`, which already powers the
  inventory dock) never reaches it — the branch that would use it is dead
  because `apply_action` has no engine handle.
- **Composition is a hunt.** Building "unlock door with key" means tabbing
  between three flat panes with no grammatical guidance; nothing indicates
  what kind of token comes next, and nothing ever submits — Enter only works
  by falling through to the story input in "passive" mode.
- **The focus model is inverted.** The dock opens *passive* (arrows don't move
  it), and the pane/story focus ring is invisible state.
- **It degrades the session while open.** The dock counts as a modal overlay,
  which hides the story prompt line (the transcript input is gated on
  `!any_modal_overlay_open()`), swallows paste, blocks v6/Glk click delivery,
  and drops graphical v6 from the pixel path to the cell path.
- **The list snapshots at open** and never refreshes across turns.

## The model: Journey's command band

Journey (Infocom v6) replaces the parser with a persistent bottom band:
columns fill in left-to-right as a phrase narrows, everything visible is
clickable, and the phrase under construction is always shown. The redesign
adopts that shape, minus the party column (babelmap has no actors to select).

```
┌ Command ─────────────────────────────────────────────────────────────────┐
│ > unlock iron door with _                                    Enter: send │
│  VERB          WHAT — here        WHAT — carried       WITH…             │
│   look          window             brass key           ▸brass key        │
│  ▸unlock       ▸iron door          lantern              lantern          │
│   open          mailbox            rope                 rope             │
│   take          leaflet                                                  │
│  n s e w · up down · in out · look inventory wait again       one-click  │
└──────────────────────────────────────────────────────────────────────────┘
```

### Decisions (user-approved 2026-08-05)

1. **Placement: bottom band.** A 6–8 row dock under the story pane, above the
   help row, columns side by side. While open it subsumes the inventory dock
   (the *carried* column IS the inventory); the inventory dock auto-hides and
   returns on close.
2. **Submission: always confirm.** A grammatically complete phrase arms the
   phrase line (`Enter: send` lights up); it is sent only by Enter or by
   clicking the phrase line. No auto-fire — including the quick-action row,
   whose picks also just fill the phrase line. (A complete-alone verb like
   `look` arms immediately, so it is still click + Enter, never a surprise
   turn.)
3. **Configurable grammar: yes.** A `[verb_menu]` config section (below).
4. **Focus: the band owns the keyboard while open.** Letters type-to-filter
   the active column, Tab jumps to the story input for free typing (the band
   stays visible), Esc clears the phrase first and closes when empty.

## Grammar: the arity table

Progressive disclosure needs each verb to declare its shape. A small internal
table (overridable via config) classifies every verb:

| Arity        | Example                | Columns offered after the verb        |
|--------------|------------------------|---------------------------------------|
| `solo`       | look, wait, inventory, n/s/e/w/up/down/in/out, again | none — phrase is complete |
| `object`     | examine, take, open, read, eat | WHAT                         |
| `object?`    | search, push, climb    | WHAT, but Enter with verb alone also valid |
| `pair(prep)` | unlock (with), put (in/on), give (to), show (to) | WHAT → WITH…/IN…/TO… (column header shows the prep) |

The active column is always the next unfilled slot; columns right of it render
dimmed until reachable. Backspace un-picks the most recent token and steps the
active column back.

## Data: live objects, per engine

- **Z-machine:** `Engine::introspect()` provides `room_objects(current_loc)`
  (here) and `contents(player_obj)` (carried). Computed where the engine
  handle is in scope — the same per-frame site that feeds the inventory dock
  (`main.rs`, `inventory_items`) — and refreshed every turn, not snapshotted
  at open.
- **Glulx / Scott (no `Introspect`):** carried = `parse_inventory_output`
  fallback; here = the transcript-scrape noun list, demoted to a single
  "WHAT — seen" group so its lower quality is visible rather than mixed in.
- The band never blocks on the engine: object columns render whatever the
  current sources give, including empty groups with a `(nothing visible)` row.

## Interaction spec

Opening: the existing `open-verb-menu` slash command and leader binding stay;
additionally a **direct default keybinding** (`F2`, rebindable — keys bind to
command strings) because leader-only is why nobody finds it.

While open (band focused):
- `←`/`→` move between columns (only across reachable ones), `↑`/`↓` within,
  PgUp/PgDn/Home/End as in every list.
- Printable characters filter the active column incrementally; Backspace with
  a filter clears it first, then un-picks tokens.
- Enter on a row picks it and advances; Enter with a complete phrase sends it
  through the ordinary `SubmitCommand` path (the phrase is plain text — the
  game parses it exactly as if typed).
- Tab moves focus to the story input (band stays, dimmed header); Tab or a
  story-pane click returns... clicking any band row refocuses the band.
- Esc: clear filter → clear phrase → close, one level per press.

Mouse: single click picks a row (and advances the column); clicking the phrase
line sends when armed; the quick-action row is one click to fill, Enter to
send, per decision 2. Wheel scrolls the hovered column. All hit rects go
through the existing `PaneRects` emit-while-drawing pattern.

## Not a modal

The band is a dock, not dialog chrome:
- It does NOT count in `any_modal_overlay_open()`. The story prompt line stays
  visible (the phrase line mirrors, not replaces, `state.input`? No — see
  below), paste keeps working, v6 stays on the pixel path, and v6/Glk click
  delivery is blocked only for clicks inside the band's own rect.
- Phrase state is band-local (`Vec<Token>`), rendered on the phrase line and
  materialized into text only at send. The story input line is left alone, so
  Tab-to-story free typing and the band coexist without fighting over
  `state.input`. (The old panel mutated `state.input` directly; that coupling
  is what forced its modal-overlay status.)

## Config

```toml
[verb_menu]
# height = 8            # band rows including frame
# auto_open = false     # open the band on story start
# verbs = [             # replaces the built-in table when set
#   { word = "unlock", arity = "pair", prep = "with" },
#   { word = "polish", arity = "object" },
# ]
# extra_verbs = [...]   # additive form; same entry shape
# quick = ["n","s","e","w","up","down","in","out","look","inventory","wait","again"]
```

`verb_dock_pct` (the left dock's width) is retired; resize mode targets the
band's height instead, mirrored to `verb_menu.height`.

## Style

New selectors (registry + template, defaults reproducing the mockup):
`verbband.phrase` (armed/disarmed variants), `verbband.column_header` /
`:active`, `verbband.quick`, `verbband.group_label`; rows reuse
`dialog.list_selected` and the panel frame reuses `panel.border[:active]`.
Every new element styleable per the CLAUDE.md rule.

## Testing

- Unit: arity table → column reachability; filter; backspace un-pick ladder;
  phrase materialization (incl. multi-word object names quoting nothing —
  games take "iron door" fine as plain words).
- Config: `[verb_menu]` round-trip, replace-vs-additive verb lists.
- Integration: a Z-machine story (Zork-era fixture in unit_tests/) — open
  band, compose take/unlock phrases from live objects, send, assert the turn
  ran; per-turn refresh (object taken moves from *here* to *carried*);
  Glulx/Scott fallback grouping; v6 pixel path NOT dropped while open;
  prompt line still drawn; inventory dock auto-hide/restore.
- Falsification discipline per CLAUDE.md for every behavioral pin.

## Out of scope (this redesign)

- Mining the game's real dictionary/grammar for its verb set (`vocabulary()`
  exists for Z-machine; a future quest could pre-filter the verb column).
- Glulx introspection (tracked separately as "SP4" in glulx_session.rs).
- Per-game verb sets in the sidecar config.
- Screen-reader linearisation of the band (SQ-0609 owns that interaction).
