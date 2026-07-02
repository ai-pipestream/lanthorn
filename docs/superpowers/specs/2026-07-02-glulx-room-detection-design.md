# Glulx Room Detection — Design Spec

**Status:** Draft for review · 2026-07-02
**Goal:** Give Glulx games live automapping by detecting the current room from
the Inform 7 **room-heading** style in the story buffer, closing the long-
deferred "SP4" gap where `GlulxSession::current_location()` always returns
`None`.

**Scope:** `app` crate only (the Glk backend + the Glulx session). `gvm` stays
zero-dependency and unchanged. No change to Z-machine detection.

---

## 1. Background — the survey

Driving 13 local Glulx stories to their first prompt with a Glk backend that
preserves the **raw Glk style class** on every output run showed one consistent
signal and one unreliable one:

| Game | Grid status window | Buffer room heading (`Subheader`) |
|------|--------------------|-----------------------------------|
| FooFoo | `Exits:` — **no room** | **Studio Apartment** |
| Superluminal Vagrant Twin | `Credits… Fuel…` — **no room** | **Orbiting Boony** |
| Coloratura | `Inside the Cellarium` | Inside the Cellarium |
| Sub Rosa | `Leathery Cliff  0/7 secrets` | Leathery Cliff |
| Dr Ludwig | `Laboratory` | Laboratory |
| TAKE | `War Chest` | War Chest |
| The Magpie | `Station` | Station |
| Zozzled | `Hotel Lobby  Exits…` | *(pre-game menu — no heading)* |
| Brain Guzzlers | `Mercury Meteor` | *(pre-game menu — no heading)* |
| And Then You Come to a House | *(empty)* | *(setup Q — no heading)* |
| THE BAT | *(empty)* | *(setup Q — no heading)* |

**Conclusion:** the **`Subheader`-styled line in the primary buffer window** is
the room name in *every* game that reached real play — including FooFoo and
Superluminal, whose status grid holds stats, not the room. The grid is only
sometimes the room, so it is **not** the primary signal. (Strategy A of the
brainstorm; grid fallback = the deferred Strategy C.)

Inform 7 prints the room title with the `Subheader` style on every room entry
and `LOOK`; menus, setup questions, and prose never use it. That makes the
signal both reliable and self-gating against pre-game screens.

---

## 2. The signal & algorithm

**Room name = the last `Subheader` line that BEGINS AT LINE START in a turn's
primary-buffer output.**

- **Style:** only `GlkStyle::Subheader` counts. `Header` is the big game-title
  banner; `Emphasized` is inline bold (FooFoo's "Knock.", Zozzled's "wish ME").
  Neither is a room.
- **Line-start only (inline-link guard):** the heading is a `Subheader` run that
  begins at the start of a line — Inform prints the room title on its own line.
  Some games also emit `Subheader` *mid-line* for inline command hyperlinks
  (Superluminal Vagrant Twin renders "credits"/"land" this way, before and after
  the real "Orbiting Boony" heading). A plain "last Subheader run" rule captures
  the trailing inline link ("land") instead of the room; requiring the run to
  begin at line start rejects inline links and selects the true heading. The
  detector tracks line-start position across the whole primary-window stream.
- **"Last line" skips the banner:** on turn one some games print the game title
  *and* the room, both as `Subheader` (Coloratura: `Coloratura` then
  `Inside the Cellarium`). Taking the **last** `Subheader` line before the
  prompt selects the room, not the title.
- **Line boundary:** a `Subheader` run (or contiguous run of `Subheader`
  characters) is terminated by a newline or by the next non-`Subheader` output.
  Trim surrounding whitespace. Ignore empty results.
- **Sticky:** Inform reprints the heading only on room change / `LOOK`. On a
  turn with no `Subheader` heading (examine, talk, failed action), the current
  room **persists** — the session remembers the last heading and reports it
  again. This mirrors the Z-machine path, which re-derives the same room every
  turn.

### Identity

Glulx's world model lives in game memory with **no standard introspection** —
there is no readable object tree like the Z-machine's. So a room is identified
**by name**: a synthetic room id derived from the normalized heading
(`crate::roomid::synthetic_room_id`, exactly as the Z-machine `NameOnly` path
already does). Two genuinely different rooms that share a name collapse to one
node. This is inherent to Glulx and accepted.

---

## 3. Components & changes

### 3.1 `AppGlk` (glk_backend.rs) — capture the heading

`put_text_attr(win, style, colour, s)` already receives the **raw
`GlkStyle`** (the render path immediately flattens it to a bold bit, losing the
`Subheader` distinction). Add a parallel, render-independent capture:

- A per-window (primary only) accumulator that appends `s` while
  `style == GlkStyle::Subheader`, and finalizes the accumulated text into a
  "last heading" slot when a non-`Subheader` run or an embedded newline ends the
  line.
- `pub fn take_room_heading(&mut self) -> Option<String>` — returns and clears
  the last finalized `Subheader` line captured since the previous call
  (None if this turn produced no heading). Drained once per turn, in lockstep
  with `take_transcript`.

The existing `log`/render runs are untouched; this is additive state.

### 3.2 `GlulxSession` (glulx_session.rs) — thread it into the turn

- Hold a sticky `last_room: Option<ObjectSnapshot>` on the session.
- In `finish_turn` (and the seed drive in `new`): call
  `self.appglk().take_room_heading()`. If `Some(name)`, build an
  `ObjectSnapshot { number: synthetic_room_id(&name), parent: 0, name }` and
  store it as `last_room`. Set `TurnResult.location = last_room.clone()` and
  `TurnResult.location_method = Some(<room-heading method>)`.
- `current_location()` returns `last_room.clone()` (was hard-coded `None`), so
  the starting room appears on the map immediately after boot.

### 3.3 Detection method — a new, *trusted* tag

The pre-game gate added for BeyondZork suppresses `LocationMethod::NameOnly`
**while the map is empty**. A Glulx game never produces an object-backed room,
so reusing `NameOnly` would suppress the *first* Glulx room **forever**. Glulx
headings must therefore carry a **distinct method** that the gate does not
touch:

- Add a room-heading method (working name `RoomHeading`; renders as
  e.g. *"via room heading"* on the map's location indicator).
- **Open implementation choice (flag for the plan):** `LocationMethod` currently
  lives in `zvm::location`. Options: (a) add the variant there (simplest, minor
  layering smell — a zvm enum naming a Glulx concept); (b) lift the
  method/indicator concept to an app-level type. Recommendation: (a) for now —
  the enum is already the app's UI vocabulary and the churn is minimal — and
  note the smell.
- `apply_turn`'s gate is unchanged: it keys only on `NameOnly`. `RoomHeading`
  flows straight through. Pre-game screens still produce **no** room because
  they emit no `Subheader` heading (self-gating), so no separate Glulx gate is
  needed.

### 3.4 Map indicator

The bottom-right location-method indicator (`show_loc_method`, styled by
`loc_indicator`) gains a label for the new method. No new config; reuses the
existing toggle and style selector.

---

## 4. Data flow (one turn)

```
gvm put_text(primary, Subheader, "Studio Apartment")
  → AppGlk.put_text_attr: render run (bold) + heading accumulator
gvm put_text(primary, Normal, "\nYou climb out of bed…")
  → AppGlk: newline finalizes heading = "Studio Apartment"
GlulxSession.finish_turn:
  take_room_heading() → Some("Studio Apartment")
  last_room = ObjectSnapshot{ id=synthetic("studio apartment"), name }
  TurnResult{ location: Some(last_room), location_method: RoomHeading, … }
apply_turn(mapper, command, result):
  method != NameOnly → observe(id, name, parse_direction(command))
```

---

## 5. Edge cases

- **Banner-only turn (title, no room yet):** e.g. a game that prints its title
  then a setup question. Title is `Header` (ignored) or a `Subheader` line
  followed by no room — last-`Subheader` may pick a subtitle. Mitigation: the
  heading is only *observed* once real play prints a room heading; a spurious
  banner subtitle is rare and, if it occurs, is a single mislabeled node
  corrected on the first real move. (If it proves common in testing, gate the
  first heading behind "a Normal room-description run followed it" — deferred
  unless a survey game needs it.)
- **Pre-game menus / setup questions** (Zozzled, Brain Guzzlers, And-Then, THE
  BAT): no `Subheader` heading → `current_location() == None` until real play.
  Verified by the survey.
- **Failed movement** (`north` into a wall): no heading reprint → sticky room
  unchanged; `apply_turn` sees the same room with a direction, exactly as the
  Z-machine path already does. No new behavior.
- **`LOOK`:** reprints the heading = same room; harmless refresh.

---

## 6. Testing

- **Unit (heading parser / AppGlk):** feed synthetic Glk run sequences —
  `Subheader "Studio Apartment"` then `Normal` text → heading "Studio
  Apartment"; title-then-room (two `Subheader` lines) → last one wins;
  `Emphasized`/`Header` runs → no heading; menu (Normal only) → None.
- **Session:** a stubbed/driven session where `take_room_heading` yields a name
  → `current_location()` and `TurnResult.location` carry the synthetic-id room;
  a heading-less turn keeps the sticky room.
- **Gate interaction:** `apply_turn` with `RoomHeading` on an empty map →
  room **is** observed (contrast the `NameOnly` gate test).
- **Story-level (ignored, local `stories/`, mirrors
  `accel_story_equivalence`):** drive each of FooFoo, Superluminal, Coloratura,
  Sub Rosa, Dr Ludwig, TAKE, Magpie to the first prompt and assert the detected
  room equals the survey's expected name; assert Zozzled / Brain Guzzlers /
  And-Then yield **no** room pre-game.

---

## 7. Non-goals / deferred

- **Grid-status fallback (Strategy C):** not needed for the surveyed games;
  revisit if a game surfaces that prints no `Subheader` heading but does put the
  room in the grid.
- **Same-name room disambiguation:** the name-based id in this spec collapses
  two genuinely different rooms that share a name into one node (and a walk
  between them registers as no move). The name alone cannot separate them, but
  this is **not** impossible in general:
  - **Connectivity-based identity (the eventual fix, engine-agnostic):** identify
    a room by its position in the movement graph rather than its name — when the
    player moves `via` a direction from a known room, a target that isn't the
    expected existing neighbour is a *new* node even if the name collides. Name
    becomes a label + sanity check. This also fixes the Z-machine `NameOnly`
    path, so it belongs to the broader mapping-redesign effort, not this spec.
    Caveats: IF maps are often non-Euclidean, so it needs a reliable "did we
    actually move?" signal and user split/merge to correct mistakes.
  - **World-model introspection (exact but costly):** read Inform's object
    tree / `location` global from Glulx game memory for a true room identity
    (the Glulx analogue of the Z-machine's PlayerParent). Research-heavy and
    version-fragile — no standard pointer to globals/objects.

  Both are out of scope here; tracked as a separate TODO. This spec ships the
  name-based v1 and is forward-compatible: only the id-derivation changes later.
- **The two "runaway" intros** (Wizard Sniffer, Mockingbird) that never reached
  a prompt in the survey: a separate boot/intro concern, not detection.
- **Aux/graphics setup prompts** ("ASCII graphics Y/N"): unaffected; they simply
  produce no room until play begins.

---

## 8. Success criteria

1. FooFoo and Superluminal — the games that motivated this — automap their
   starting room (`Studio Apartment`, `Orbiting Boony`) and subsequent rooms.
2. The other surveyed play-reaching games detect their rooms.
3. Pre-game menus/setup screens create no false room.
4. Z-machine detection and the BeyondZork gate are untouched (regression-free).
5. `gvm` remains zero-dependency.
