# Verb panel redesign: the command band

**Status:** implemented (SQ-0664), amended (SQ-0667, 2026-08-05 — see
"Amendments" below) after the first real play session, then amended again
(SQ-0675, 2026-08-05 — see "Amendments (SQ-0675 — layout follow-up)" further
down) for two user-requested layout tweaks once the band had been played a
bit longer: the VERB column's header row was pure blank space, and the flat
quick row wasted the band's whole width on one row when a spatial compass
block would read faster and cost less height. Shipped as the
**command band** — the feature was renamed from "verb menu"/"verb panel"
throughout during implementation, so read every "verb menu" below as the
command band: the slash command is `open-command-band`, the config section
`[command_band]`, the style selectors `band.*` (not `verbband.*`), the code
lives in `render/command_band.rs` + `CommandBandState`, and the resize
target is `ResizeTarget::CommandBand`.

Two refinements the implementation settled that the design left open:

* **~~The phrase line is the last stop in the focus ring~~, to the right of
  the reachable columns.** RETIRED by the 2026-08-05 amendments below — there
  is no more phrase line, so no more trailing ring stop. Kept here struck
  through rather than deleted, since the *reason* it existed still matters:
  it was what kept "always confirm" honest for an `object?` verb like
  `search`, complete the moment it's picked but still able to take an object.
  See "Amendments" for how that guarantee survives without it.
* **A quick-row word that is not in the verb table counts as `solo`.** The
  shipped quick row spells the compass `n`/`s`/`e`/`w` while the verb table
  spells them out, and a quick action IS the whole command. (Still true, and
  now also why the compass survives the VERB-column exclusion below — see
  Amendment 2.)

Supersedes
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

**This mockup is the ORIGINAL design, kept for history.** The 2026-08-05
amendments below retired the frame, the phrase line, and the VERB column's
header text — see "Amendments" for the shape the band actually ships in.

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

The as-shipped (post-amendment) band, for comparison — no frame, no phrase
line (that IS the real `>` prompt above it now, drawn by the story pane, not
the band), no VERB header text:

```
> unlock iron door with _
                 WHAT — here        WHAT — carried       WITH…
  look            window             brass key           ▸brass key
 ▸unlock         ▸iron door          lantern              lantern
  open            mailbox            rope                 rope
  take            leaflet
 n s e w · up down · in out · look inventory wait again
```

**Superseded again by SQ-0675 (2026-08-05, same day, later in the session):**
this mockup still shows the VERB column's header row blank and the quick
words as a single flat bottom strip — both retired by the "Amendments
(SQ-0675 — layout follow-up)" section further down. VERB's list now starts
on what was this blank row (one more visible verb, no separate header for
it), and the bottom strip becomes a left-anchored compass-rose block
whenever the band is wide enough; the flat strip shown here survives only as
the narrow-band fallback.

### Decisions (user-approved 2026-08-05)

1. **Placement: bottom band.** A 6–8 row dock under the story pane, above the
   help row, columns side by side. While open it subsumes the inventory dock
   (the *carried* column IS the inventory); the inventory dock auto-hides and
   returns on close. (Amended same day: no frame, so the row budget dropped —
   see Amendment 3; the "6–8 row" figure describes the pre-amendment band.)
2. **Submission: always confirm — except the quick row.** A grammatically
   complete phrase arms the phrase line (`Enter: send` lights up); it is sent
   only by Enter or by clicking the phrase line. No auto-fire from any column
   pick or composed phrase. (A complete-alone verb like `look`, picked from
   the VERB column, arms immediately, so it is still click + Enter, never a
   surprise turn.)

   **Amendment (2026-08-05, user feedback after first use):** the quick row
   (`n s e w up down in out look inventory wait again`) is a deliberate
   exception, added after the very first play session made the extra
   confirm on that row feel like pure friction. Clicking or keyboard-picking
   a quick-row entry submits it AT ONCE — no Enter. Every quick word is
   already a complete command on its own (this is the same fact the second
   refinement above already relies on — "not in the verb table ⇒ solo"), so
   the second confirm bought nothing but an extra step on the one row built
   for single-click speed. Column picks and composed phrases are UNCHANGED —
   they still always-confirm exactly as decided above; this exception is
   scoped to the quick row alone. A quick pick also never touches an
   in-progress phrase (pinned choice, Amendment 1 below): it is an
   interjection — glancing with `look` mid-`unlock iron door` — not a
   composition step, so the phrase is exactly as it was once the quick
   command returns.
3. **Configurable grammar: yes.** A `[command_band]` config section (below).
4. **~~Focus: the band owns the keyboard while open.~~** ~~Letters
   type-to-filter the active column, Tab jumps to the story input for free
   typing (the band stays visible), Esc clears the phrase first and closes when
   empty.~~ **RETIRED by SQ-0676** (2026-08-05, third feedback round of the same
   day) — the model inverted: typing ALWAYS reaches the story prompt and the
   band reads it back. Struck through rather than deleted because it is what
   every "while the band is focused…" sentence below was written against; see
   "Amendments (SQ-0676 — typing always wins)" for the gesture table that
   replaced it.
5. **The VERB column excludes quick words** (2026-08-05 amendment, same
   feedback round as decision 2). Showing `look`/`wait`/`again`/etc. in both
   places is redundant — the VERB column filters out any word present in the
   *effective* quick list (the user's configured `quick` when set, else the
   built-in row), so removing a word from a custom `quick` puts it back in
   the VERB column and adding one takes it out. The compass survives under
   the default config only because the two rows spell it differently (quick:
   `n s e w`, table: `north south east west`) — that is refinement 2 at the
   top of this document ("not in the table ⇒ solo"), not a new rule.

### Amendments (2026-08-05, user feedback after first use)

Everything below landed the same day, in the same feedback round, once the
band was actually played rather than just read about. Amendments 1 and 2 are
decisions 2 and 5 above (cross-referenced here so this is a single place to
scan the whole batch); 3–7 don't have a numbered decision of their own.

1. **Quick-row picks fire immediately** (decision 2's amendment) — no Enter,
   scoped to the quick row only. **Pinned choice for an in-progress phrase:**
   leave it intact. The quick row is an interjection (glancing with `look`
   mid-`unlock iron door`), not a composition step, so nothing about a
   partially composed phrase changes when a quick word fires.
2. **The VERB column excludes quick words** (decision 5) — config-aware,
   following the effective `quick` list.
3. **The band's frame is retired.** No top/bottom border rows, no "Command"
   title strip — it is a borderless fill now (`render/command_band.rs`
   still paints an opaque background so panes behind it never show through
   mid-slide, and picks up `panel.border:active`'s style across the WHOLE
   fill, not just a border, as its resize-mode affordance — there is no
   border left to accent). The row budget dropped by the frame's 2 rows;
   `DEFAULT_BAND_ROWS` moved from 8 to 5 to show exactly as many list rows as
   before (`MIN_BAND_ROWS` 5→3, `MAX_BAND_ROWS` 14→11, same 3-row delta
   throughout).
4. **The band's own phrase line is retired.** Composing now happens DIRECTLY
   on the real story input line (`state.input`) — see "Not a modal" below for
   the mechanics. This ate the phrase line's own row (part of the same 3-row
   budget cut as the frame in #3) and retired the `Enter: send` affordance:
   "always confirm" is now simply "it's the normal prompt" — Enter on the
   band still only ever picks (`Action::BandEnter`, unconditionally now, no
   more phrase-line branch), and sending is the ordinary Enter on the real
   input line once the player Tabs over to it. **No replacement "armed"
   indicator was added** to the input line rendering — the `band.phrase` /
   `band.phrase:armed` selectors are retired (removed from the theme
   registry) rather than repointed at the prompt, on the "keep it simple"
   allowance: a styled-prompt affordance would have meant reaching into
   `render/screen.rs`'s prompt-line rendering for a fairly marginal payoff
   (the composed text is already visibly sitting at the prompt, which is
   itself a strong enough "this is what will send" signal).
5. **The VERB column header carries no text.** "VERB" was the only column
   whose header didn't actually convey information (WHAT — here / WHAT —
   carried / WITH… all do); a bare column of verbs is self-evident. The
   header ROW is unchanged for every column (still 1 row, still clickable to
   focus that column) — only the VERB column's label text was dropped; the
   active-column marker (`▸`) still shows there on its own.
6. **Empty object columns say so explicitly.** Following the SQ-0668 fix that
   gave the carried/here columns real data (a genuinely empty column is now a
   real possibility, not just a scrape-fallback artifact), a blank list reads
   as broken. The `(nothing visible)` placeholder this design already
   specified is now column-specific: `(nothing here)` / `(nothing carried)`
   for those two columns, `(nothing visible)` stays the fallback for
   VERB/SECOND (which are never genuinely this empty in practice).
7. **The player object is excluded from the *here* column, by id.** It is
   structurally a child of whatever room the player is in, so without this it
   showed up in every room of every game (Zork 1: "cretin"). Filtered by
   object id (`Introspect::room_objects_excluding`, matched against
   `player_obj`/`introspect().player_object()`), deliberately not by matching
   the printed name — a real scenery object could coincidentally be called
   the same thing the game calls the player. `examine me` still works by
   typing it; there is no dedicated row for it.

### Amendments (2026-08-05, SQ-0675 — user-requested layout follow-up)

Two more layout tweaks, requested after playing with the SQ-0667 band for a
while — smaller than the SQ-0667 batch above, but landed the same day, so
they get their own dated subsection rather than being folded into it.

1. **The VERB column's header row is reclaimed as an extra list row.**
   Amendment 5 above dropped the "VERB" label but left the header ROW itself
   drawn — a blank, unclickable strip in a band that is deliberately compact.
   `render/command_band.rs::draw_column` now special-cases `COL_VERB`: no
   header row is drawn or hit-tested for it at all, and its list starts at
   the column's very first row instead of the second. Concretely, at the
   same column height, the VERB column shows exactly **one more visible
   verb** than an object column shows items (an object column still spends a
   row on a real header — WHAT — here / WHAT — carried / WITH… all carry
   information worth a label). The per-row selection marker (`▸`) already shown on the
   highlighted item is what now carries the "this column has keyboard focus"
   signal for VERB, since there is no header left to show it separately.
   `hits.headers` therefore has `BAND_COLS - 1` entries, not `BAND_COLS` —
   VERB contributes none; clicking its reclaimed top row picks the first
   verb there, exactly like clicking any other row.

2. **The flat quick row becomes a compass-rose block when there's room.**
   The one-click quick row (`n s e w up down in out look inventory wait
   again` by default) used to be a single strip spanning the band's full
   width along the bottom, costing the columns one row of height. It is
   replaced by a spatial block anchored to the band's LEFT edge whenever the
   band is wide enough:

   ```text
    NW  N  NE      up down in out
     W  ·  E       look inventory
    SW  S  SE      wait again
   ```

   (This is the actual layout the shipped algorithm produces for the
   built-in `quick` list — not the same word grouping as an earlier
   illustrative sketch, but the same idea: a 3×3 compass rose with an inert
   centre, plus everything else in the effective quick list packed beside it
   into the same 3 rows via a greedy row-major wrap, tightest column first.)

   - **What lands in the rose.** A quick word is routed into one of the 8
     outer rose cells by the DIRECTION it names (`mapper::direction::parse_
     direction`), not its spelling — the same "match by meaning" rule
     Amendment 5 above already uses for the VERB-column exclusion, so a
     custom `quick` spelling `"north"` still lands in the rose's N cell. Only
     the 8 compass POINTS (N/S/E/W/NE/NW/SE/SW) are rose cells; `up`/`down`/
     `in`/`out` are directions too but not compass points, so they flow as
     ordinary words instead — this is deliberate, not an oversight (see
     `split_quick_rose`'s doc). The centre cell is always inert decoration,
     styled with the map matrix's own frontier/dim style
     (`map.matrix.cell:frontier`) rather than a new selector.
   - **What lands in the words.** Every effective quick word that isn't a
     rose cell — under the default `quick`, that's `up down in out look
     inventory wait again` — flows left-to-right, wrapping to a new row only
     when it would overflow, packed into a width computed to be the
     NARROWEST one that still fits everything in the rose's fixed 3 rows
     (`word_flow_width`). No magic constant: the width is derived from the
     actual word lengths, same spirit as the fallback threshold below.
   - **A `quick` list with no compass words draws no rose at all** — just the
     flowing word list, starting where the rose's margin would otherwise
     have been, so nothing is wasted on an empty diagram.
   - **Width policy, no config knob.** `draw_command_band` computes the
     block's exact width (`quick_block_layout`) and compares it against the
     band's actual content width plus the same "6 cells per column" minimum
     `column_rects` already enforces; below that, it falls back to the
     original SQ-0667 flat row along the bottom (unchanged in every other
     respect — same immediate-submit semantics, same `hits.quick` shape).
     There is no `[command_band]` setting for this threshold — it is derived
     from real geometry (rose width + word-flow width + minimum column
     width), per CLAUDE.md's "compute the threshold from actual widths"
     guidance for exactly this kind of layout decision.
   - **Height budget: the block sits BESIDE the columns, not above them.**
     Unlike the flat row, the block does not cost the columns any height at
     all — it is always exactly 3 rows tall regardless of the band's
     configured height, occupying the band's top-left corner, while the
     columns render at the band's FULL height to its right. A band taller
     than 3 rows leaves the block's own strip blank below row 3; the
     alternative (stretching a compass rose to fill an arbitrarily tall
     band) has no sensible layout of its own, so this was chosen as the
     simplest option that never shrinks the columns.
   - **Every cell is still a `hits.quick` entry** — `(index into
     `band.quick`, rect)` — identical in shape to the flat row's own hit
     rects, so `input::band_quick_pick_command` and `main::band_mouse_action`
     needed NO changes: this is a different LAYOUT of the same one-click
     submit contract, not a new one. Directions still spell out in full on
     submission (`nw` sends `northwest`); the rose only abbreviates the
     DISPLAY, as the pre-existing quick-row behavior already did.
   - **Keyboard reachability is unchanged.** The block (rose or flat row,
     whichever is showing) was never part of the `←`/`→` column ring and
     still isn't — it is a mouse/one-click surface only, exactly as the flat
     row always was.

### Amendments (2026-08-05, SQ-0676 — typing always wins)

**The focus model inverts.** Decision 4 above ("the band owns the keyboard
while open") is RETIRED, and with it every key the band took from the story
prompt. The band is now a **reactive suggestion surface**: it never owns a
keystroke that could be text, it READS the prompt, and it offers one armable
quick selection for the keys that are left. This was the third feedback round
of the same day, and it came from the plainest possible symptom — with the
band open, typing `w` and pressing Enter did nothing at all: the letter went
into a column filter and Enter picked a row. A band you cannot type past is a
band you close.

**The gesture table, as shipped.** The band is open throughout; "armed" means
the last thing pressed was an arrow, "typing" means the last thing pressed
changed the text on the prompt (a character, Backspace, a delete, a paste, a
completion).

| Key | Armed | Typing / neutral |
|---|---|---|
| any printable char | types at the prompt (and DISARMS) | types at the prompt |
| Backspace / Ctrl+W / … | edits the prompt (and DISARMS) | edits the prompt |
| paste | inserts at the prompt (and DISARMS) | inserts at the prompt |
| `↑` `↓` `←` `→` (plain) | move the quick highlight — spatially in the rose+word block, linearly in the flat fallback | ARM the highlight on the first quick word |
| Shift+`←`/`→`/`↑`/`↓` | pans the map, unchanged | pans the map, unchanged |
| Enter | fires the highlighted quick word (immediate submit, direction spelled out) | submits the prompt **exactly as typed** |
| Tab / Shift-Tab | fires the highlighted quick word | completes the current word to the band's nearest match; consumed no-op when nothing matches |
| Esc | disarms the highlight | closes the band |
| Ctrl/Alt chords, the leader prefix | fall through, unchanged | fall through, unchanged |
| mouse | unchanged (click picks / advances / fires quick; wheel scrolls a column) | unchanged |

Everything else follows from that table:

1. **Type-to-filter is retired.** The columns never narrow. The filter it
   replaced is now the word being typed AT THE PROMPT, applied as a passive
   **nearest-match highlight** (`CommandBandState::nearest_match`): prefix
   match first, then a prefix on any word of a multi-word name (`do` finds
   `iron door`), then a substring; earliest expected column and earliest row
   break ties. No match ⇒ no highlight, and Tab does nothing.
2. **The phrase state is fed by the typed line**
   (`CommandBandState::sync_from_input`, called from `apply_action`'s wrapper
   whenever `state.input` changed). The parse anchors on the FIRST typed token
   that is a table verb — text before it is the player's own (`well, take
   mailbox`) — and everything after fills the object slot, split at the pair
   verb's preposition once that is typed. It never counts tokens: object names
   are routinely multi-word. The word still under construction counts as a
   token for the GRAMMAR (a bare `take` is a chosen verb — it is exactly what a
   click leaves on the prompt) but not for the highlight, which is what that
   word is matching against. So `take ` opens the object columns whether it was
   typed or clicked, and `unlock door with ` moves the expectation to WITH…
3. **Every arrow drives the quick block**, which is why command history (plain
   `↑`/`↓`) is reachable only with the band CLOSED — a deliberate trade, and
   the one thing this amendment takes away. Arrowing arms; movement clamps
   rather than wraps (the rose is a diagram, and wrapping off its west edge to
   its east one would contradict what it draws); the flat fallback row
   navigates linearly. The renderer tells the input layer which of the two it
   drew (`CommandBandState::quick_flat`, the same render-informs-input
   handshake `input_text_origin` uses for the caret).
4. **Tab reconciliation with the dictionary autocomplete:** while the band is
   open, the band's suggestion IS the completion source, full stop. It takes
   precedence over `Action::Autocomplete`, and when the band has no match Tab
   is a consumed no-op rather than a silent hand-off to the dictionary — one
   completion source at a time is the only version of this that a player can
   predict. Closed, Tab is unchanged in every respect (dictionary
   autocomplete, then ToggleFocus). The visible consequence: slash-name
   completion (`/sav`+Tab) wants the band closed.
5. **The Esc ladder loses two rungs**: the filter rung retired with the filter,
   and the phrase rung retired because the phrase is now the player's own typed
   line — Esc must never delete text from the prompt. Disarm, then close.
6. **The column focus ring is retired** (`←`/`→` across columns, `↑`/`↓`
   within, PgUp/PgDn/Home/End, and `Action::BandEnter`'s "pick the highlighted
   row"). There is no keyboard column focus left to draw either: a column
   header lights when the GRAMMAR expects it, and exactly one row — the nearest
   match — wears the `▸` marker. `story_focused` and `Action::BandToggle
   StoryFocus` are gone; so are `BandNav`/`BandFilterChar`/`BandBackspace`/
   `BandFocusBand`. Clicking a column row still composes onto the prompt
   exactly as before.
7. **The mouse contract is untouched.** It was never the problem the inversion
   was solving.
8. **The four diagonals join the default quick list** (`ne`/`nw`/`se`/`sw`,
   between `w` and `up`), so the rose the SQ-0675 amendment drew has no
   permanently empty cells. Games with no diagonal vocabulary (classic Scott
   Adams) answer "I don't understand", exactly as they would to the same word
   typed by hand — no gating on the engine, which would be a lie about what the
   parser accepts.

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

- **Z-machine:** `Engine::introspect()` provides
  `room_objects_excluding(current_loc, player_obj)` (here — see Amendment 7
  above: the player's own object is filtered out by id) and
  `contents(player_obj)` (carried). Computed where the engine handle is in
  scope — the same per-frame site that feeds the inventory dock (`main.rs`,
  `inventory_items`) — and refreshed every turn, not snapshotted at open.
- **Glulx / Scott (no `Introspect`):** carried = `parse_inventory_output`
  fallback; here = the transcript-scrape noun list, demoted to a single
  "WHAT — seen" group so its lower quality is visible rather than mixed in.
- The band never blocks on the engine: object columns render whatever the
  current sources give, including empty groups — `(nothing here)` /
  `(nothing carried)` for the object columns specifically, `(nothing
  visible)` elsewhere (Amendment 6 above).

## Interaction spec

Opening: the existing `open-command-band` slash command (renamed from `open-verb-menu`) and leader binding stay;
additionally a **direct default keybinding** (`F2`, rebindable — keys bind to
command strings) because leader-only is why nobody finds it.

While open — **superseded by SQ-0676's gesture table above**; the bullets below
describe the pre-inversion band and are kept for the history of what each key
used to mean:

- ~~`←`/`→` move between columns (only across reachable ones — Amendment 3/4
  retired the trailing phrase-line ring stop, so the ring now clamps at the
  last reachable column instead of continuing on to a send state), `↑`/`↓`
  within, PgUp/PgDn/Home/End as in every list.~~ Every plain arrow drives the
  quick block now; the column ring is gone.
- ~~Printable characters filter the active column incrementally; Backspace with
  a filter clears it first, then un-picks tokens — which, since Amendment 4,
  also removes that token's own contribution from the tail of the real story
  input line (falling back to an ordinary single-character backspace if that
  tail has since diverged, e.g. the player free-typed past it after Tab-ing
  over).~~ Both edit the prompt now, exactly as with the band closed; the
  phrase state re-derives from the line afterwards, which is what replaced the
  un-pick ladder.
- ~~Enter on a row picks it and advances — ALWAYS, unconditionally, per
  Amendment 4~~ Enter fires the armed quick word, or submits the prompt as
  typed. Composing by MOUSE is unchanged: a click appends the picked token onto
  the real story input line (`state.input`), merging with whatever free text
  was already there rather than overwriting it; sending is the ordinary Enter
  on THAT line — same `SubmitCommand` path a typed command takes, the phrase is
  plain text and the game parses it exactly as if typed.
- ~~Tab moves focus to the story input (band stays, dimmed header); Tab or a
  story-pane click returns... clicking any band row refocuses the band.~~ Tab
  fires the armed quick word, else completes the current word to the band's
  nearest match. There is no focus to move anymore.
- ~~Esc: clear filter → clear phrase (Amendment 4: also strips that phrase's
  contribution back out of the real input line, if the tail still matches
  unmodified) → close, one level per press.~~ Esc: disarm → close.

Mouse: single click picks a row (and advances the column, composing onto the
real input line per Amendment 4); the quick-action row is one click to SEND —
AT ONCE, no Enter (2026-08-05 amendment to decision 2/Amendment 1 above; was
originally one click to fill, Enter to send). Wheel scrolls the hovered
column. All hit rects go through the existing `PaneRects` emit-while-drawing
pattern. (There is no more phrase-line hit rect — Amendment 4 retired it
along with the row itself.)

## Not a modal

The band is a dock, not dialog chrome:
- It does NOT count in `any_modal_overlay_open()`. The story prompt line stays
  visible, paste keeps working, v6 stays on the pixel path, and v6/Glk click
  delivery is blocked only for clicks inside the band's own rect.
- **Superseded by Amendment 4 (2026-08-05):** the paragraph below described
  the original, pre-amendment model — kept for history, corrected inline
  rather than deleted, since it explains exactly what changed and why.

  ~~Phrase state is band-local (`Vec<Token>`), rendered on the phrase line
  and materialized into text only at send. The story input line is left
  alone, so Tab-to-story free typing and the band coexist without fighting
  over `state.input`. (The old panel mutated `state.input` directly; that
  coupling is what forced its modal-overlay status.)~~

  Grammar bookkeeping is still band-local (`CommandBandState::picks` — arity,
  column reachability, and `phrase_text()` are still computed exactly as
  before), but the composed text itself is no longer rendered on a
  band-local row: it is APPENDED directly onto the real `state.input` on
  every successful pick. This is the ONE piece of the original "dock, not
  modal" argument that flipped — the band now touches `state.input` on
  purpose, deliberately merging with whatever the player already typed
  rather than fighting it: a pick's contribution is tracked by its rendered
  text so a later re-pick or un-pick can find and replace/remove exactly
  that tail (`input::sync_band_phrase_to_input` / `strip_band_tail`,
  called from `apply_action`, since `CommandBandState` itself has no
  sibling-field access to `state.input` — it stays a pure, engine- and
  app-state-agnostic type). If the input's tail has since diverged from what
  the band composed — the player Tab-ed over and typed something after it —
  a pick APPENDS instead of clobbering: the band only ever edits the part of
  the input line it itself put there. This is what keeps Tab-to-story free
  typing and the band coexisting: they now share one buffer by design,
  instead of two buffers that never touch. The quick row is the one
  exception in the other direction — it composes nothing onto `state.input`
  at all (Amendment 1), consistent with being an interjection rather than
  part of the composed phrase.

## Config

```toml
[command_band]
# height = 5             # band rows (no frame since Amendment 3 — every row is content)
# auto_open = false     # open the band on story start
# verbs = [             # replaces the built-in table when set
#   { word = "unlock", arity = "pair", prep = "with" },
#   { word = "polish", arity = "object" },
# ]
# extra_verbs = [...]   # additive form; same entry shape
# quick = ["n","s","e","w","ne","nw","se","sw","up","down","in","out","look","inventory","wait","again"]
```

`height`'s default moved from 8 to 5 (Amendment 3/4: the frame and the phrase
line together were 3 of the original 8 rows of chrome); `MIN_BAND_ROWS`
5→3, `MAX_BAND_ROWS` 14→11, same delta throughout.

`verb_dock_pct` (the left dock's width) is retired; resize mode targets the
band's height instead, mirrored to `command_band.height`.

## Style

New selectors (registry + template, defaults reproducing the mockup):
`band.column_header` / `:active`, `band.quick`, `band.group_label`; rows
reuse `dialog.list_selected`. Every new element styleable per the CLAUDE.md
rule.

**Retired 2026-08-05 (Amendments 3–4):** `band.phrase` and
`band.phrase:armed` are gone — removed from the theme registry outright
rather than repointed, since the phrase line they styled no longer exists
and no replacement "armed" indicator was added (see Amendment 4's "keep it
simple" note). The panel frame's `panel.border[:active]` reuse is also gone
— the band draws no frame anymore; `panel.border:active`'s STYLE is instead
applied to the band's whole fill as its resize-mode affordance (not reused
as a *selector* the band exposes, just borrowed for one color).

**No new selectors for SQ-0675 (2026-08-05):** the compass-rose block reuses
`band.quick` for the rose's outer cells and the flowing words (same selector
the flat row it can replace already used) and `map.matrix.cell:frontier` for
the inert centre dot — that selector already existed for exactly this
"unexplored/inert, dimmed out of the way" role on the map's own matrix view.
Nothing new was added to the theme registry or template.

## Testing

- Unit: arity table → column reachability; filter; backspace un-pick ladder;
  phrase materialization (incl. multi-word object names quoting nothing —
  games take "iron door" fine as plain words).
- Config: `[command_band]` round-trip, replace-vs-additive verb lists.
- Integration: a Z-machine story (Zork-era fixture in unit_tests/) — open
  band, compose take/unlock phrases from live objects, send, assert the turn
  ran; per-turn refresh (object taken moves from *here* to *carried*);
  Glulx/Scott fallback grouping; v6 pixel path NOT dropped while open;
  prompt line still drawn; inventory dock auto-hide/restore.
- Falsification discipline per CLAUDE.md for every behavioral pin.
- **Added 2026-08-05 (SQ-0667, the amendments above):** a quick pick fires
  immediately (pinned against the pre-amendment fill-only behavior, which the
  band test suite already had a pin for — flipped deliberately, not deleted)
  and never mutates the band's in-progress phrase; the VERB column excludes
  the effective `quick` list, config-aware in both directions (add a word to
  `quick` → gone from VERB; remove it → back); column picks and composed
  phrases still always-confirm (regression pin, unchanged); the frame and
  phrase line no longer render (no border glyphs, no "Command" title, no
  "Enter: send"); the VERB column shows no header text while WHAT/WITH keep
  theirs; an empty carried/here column says `(nothing carried)` /
  `(nothing here)` explicitly; the player object is excluded from *here* by
  id, verified by diffing against the unfiltered `room_objects` call rather
  than asserting a specific printed name (games vary — Zork 1 prints
  "cretin", the minizork fixture used in the test prints "you"). Every new
  pin was revert-verified per CLAUDE.md: temporarily undoing the
  corresponding source change and confirming the pin fails with the
  originally reported symptom before trusting it.
- **Added 2026-08-05 (SQ-0675, the layout follow-up above):** the VERB
  column's reclaimed header row shows exactly one more visible verb than an
  object column shows items, at the same column height, and contributes no
  `hits.headers` entry (`BAND_COLS - 1`, not `BAND_COLS`); the compass rose
  renders all 8 outer cells with real hit rects when a custom `quick` list
  supplies all 8, each resolving through the same submission lookup the flat
  row always used (directions spell out in full); the word flow holds
  exactly the effective quick words that are NOT one of the 8 compass points
  (`up`/`down`/`in`/`out` stay words, not rose cells); a `quick` list with no
  compass words draws no rose at all, just the word list; and a band too
  narrow for the block falls back to the original flat row unchanged. Every
  pin was revert-verified the same way as the SQ-0667 batch: temporarily
  undoing the corresponding source change and confirming the pin fails
  first.

- **Added 2026-08-05 (SQ-0676, the focus inversion above):** the headline pin
  is that typing `w` at an open band puts `w` on the story prompt and Enter
  submits it (it fails against HEAD, where the intercept ate both); plus
  arrows arm and move the quick highlight spatially through the rose, armed
  Enter and armed Tab both fire the highlighted quick word, typing disarms so
  Enter submits the text rather than a stale highlight, Tab after typing
  completes to the nearest match AND beats the dictionary autocomplete while
  the band is open (with closed behaviour re-pinned in the same test), an
  unmatched Tab is a no-op, Esc disarms then closes and never eats the prompt,
  a mouse click still picks, a paste still reaches the prompt, and the typed
  word's highlight renders on exactly one row (none when nothing matches). The
  retired-mode tests were FLIPPED, not deleted — `typing_filters_the_active_
  column`, `backspace_clears_the_filter_then_unpicks`, `escape_ladder_filter_
  then_phrase_then_close`, `tab_swaps_focus_without_closing_the_band`,
  `focus_ring_skips_unreachable_columns`, `up_down_move_the_active_column_
  selection`, `letters_filter_when_the_band_is_focused_and_type_when_it_is_
  not`, `command_band_tab_still_swaps_focus` and the paste split each became
  the pin for the behaviour that replaced it. An integration test drives the
  whole path against a real story's live object tree (type three characters of
  a real object's name, Tab, get its full multi-word name on the prompt). The
  default quick list's four new diagonals are pinned in the rose (all eight
  cells occupied) and in the submission lookup (`ne` → `northeast`).

## Out of scope (this redesign)

- Mining the game's real dictionary/grammar for its verb set (`vocabulary()`
  exists for Z-machine; a future quest could pre-filter the verb column).
- Glulx introspection (tracked separately as "SP4" in glulx_session.rs).
- Per-game verb sets in the sidecar config.
- Screen-reader linearisation of the band (SQ-0609 owns that interaction).
