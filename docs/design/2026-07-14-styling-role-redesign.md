# Styling redesign: role palette + full Glk namespace (SQ-0309)

**Status:** design spec (decided in brainstorm 2026-07-14). Implement AFTER the
current Glk-borders/styling branch merges — it touches the same styling code.

## Goal

Collapse the ~50 independently-declared styleable elements (spread across 5
duplicated selector enumerations) into a small **role palette** that everything
inherits from, while giving Glk styling full first-class support. A theme author
sets ~7 colours and gets a coherent look; power users override any single element
or Glk slot. One registry drives parse / resolve / export / editor, retiring
`SELECTOR_FIELDS` / `SELECTOR_GROUPS` / `style_for_selector` / `apply_color_decls`
/ `write_style_full` duplication.

Two levers, both used (user: "both — roles + prune dead"):
- **Inherit more:** every element = `parent + delta`, so most need no explicit value.
- **Exist fewer:** collapse near-duplicates onto roles + retire elements nobody restyles.

Builds directly on the shipped SQ-0331 (per-Glk-style `glk_styles`) and SQ-0319
(garglk.ini import) and their resolution chain — this redesign does not change
the cascade, it makes the slots first-class and the app elements derivations.

## The model

### 1. Roles — the ~7 roots a theme actually sets
`text · chrome · border · accent · muted · alert · heading`
Each carries fg / bg / modifiers. (User chose "balanced".)

| Role | meaning |
|---|---|
| text | body ink on the page |
| chrome | ink on a UI surface (bars/panels/upper window) |
| border | lines/frames |
| accent | highlight (links, selection, current, badges) |
| muted | dim / secondary (labels, meta, suggestions) |
| alert | warning / error |
| heading | emphasized titles / headers |

### 2. App elements — derivations of roles (`parent + delta`)
| Element(s) | parent | delta |
|---|---|---|
| transcript | text | — |
| status_bar, help_bar, upper_window, story_info | chrome | — |
| status_header, story_title | heading | on chrome |
| map_border, story_border, upper_window_border, input_line, suggestion_line, dialog box, scrollbar | border | (role/side deltas) |
| room_current, map_layer_tab_active, transcript_location | accent | — |
| room_selected, story_badge | accent | reverse |
| hyperlink | accent | underline |
| story_info_label, suggestion, transcript_meta, transcript_system | muted | — |
| transcript_warning | alert | — |
| transcript_crash | alert | bold |

**Pruning (two kinds):**
- *Auto-collapse:* any element whose default already equals its role becomes a
  pure derivation (no independent declaration; zero visual change). Audit the
  default theme; likely most elements.
- *Retire:* shortlist to remove entirely (bring to user to veto) — candidates:
  `meta_marker` / `warning_marker` (as separate from the text they mark),
  `story_info_cover`, `transcript_system` (merge into `transcript_meta`).

### 3. `glk.*` — the full Glk-style namespace (first-class, defaults to roles)
Every Glk style × window type is an addressable selector
(`glk.buffer.<style>`, `glk.grid.<style>`) that **defaults to a role
derivation** — so authors set nothing and get coherence, but can override any
one slot for full Glk fidelity ("full support for glk" — user).

Buffer defaults:

| Glk style | default | | Glk style | default |
|---|---|---|---|---|
| Normal | text | | Note | muted |
| Emphasized | text + italic | | BlockQuote | muted |
| Preformatted | text + mono | | Input | accent *(OPEN)* |
| Header | heading | | User1/User2 | text |
| Subheader | heading | | Alert | alert |

Grid styles derive the same way but on **chrome** as their base *(OPEN: chrome
vs text for grid Normal)*. Open judgment calls flagged: Input (accent vs text),
Note/BlockQuote (muted vs text), grid base.

### 4. `map.*` — OPEN DESIGN AREA (user flagged "other options")
The map is a distinct visual domain (room fills vs borders, edge colours,
current/selected/visited distinction, per-layer colour coding, distortion
markers) that does not map cleanly onto the text-oriented roles. Give it its own
`map.*` sub-palette. Options to decide in a follow-up:
- **(a) Own roots:** a small independent map palette (room, room-current,
  room-selected, connector, layer-cycle, map-border) — most control, more knobs.
- **(b) Reference shared roles:** map elements derive from the shared roles
  (room-current → accent, connector → border/muted) with map-specific deltas —
  fewer knobs, coupled to the app palette.
- **(c) Hybrid:** map has its own *tokens* (fills/edges/layer cycle) but current
  /selection reference the shared `accent` so highlight colour stays consistent.
Recommendation: (c). Decide during implementation; not blocking the role palette.

### 5. Resolution chain (unchanged from SQ-0331 — do not re-litigate)
Per channel, most-specific wins: garglk per-stream override → game stylehint
(per-window, SQ-0328 hybrid) → `glk.*` slot → its role → terminal. Gate:
`honor_game_colours` on → normal; off → drop the game-stylehint layer
(`slot.or(role)`). garglk.ini imports into the `glk.*` slots (SQ-0319).

## Registry (the implementation core)
One declarative table; each row: `name`, editor group, kind (style /
border+glyphs / placement), parent (role or another selector), optional delta,
default. A single resolve pass computes the flat theme map (roles → element
derivations → glk defaults → user decls → per-game overlay → garglk.ini). The
table drives: parsing, resolution with parent fallback, editor groups,
`/dump`/export round-trip. This deletes the 5 duplicated enumerations.

## TOML schema (breaks the old one — pre-release, no shim)
```
[roles]           # the ~7 roots
text   = { fg = "...", bg = "..." }
accent = { fg = "..." }
...
[elements]        # overrides only; unset = role derivation
hyperlink = { parent = "accent", underline = true }
[glk.buffer]      # overrides only
header = { parent = "heading" }
[glk.grid]
alert  = { fg = "..." }
[map]             # per the chosen map option
[symbols] / transcript rules  # carried over
```

## Migration & compatibility
Pre-release: the style.toml schema breaks; ship regenerated default themes +
`style.example`; README notes old files aren't migrated. No back-compat decoder.

## Open questions (for implementation)
1. Map role model — options (a)/(b)/(c) above (user flagged).
2. Glk defaults: Input (accent vs text), Note/BlockQuote (muted vs text), grid base.
3. Final retire list (auto-collapse is safe; retirals need a veto).
4. Whether a separate raw-colour "palette" tier sits under the roles (change
   "accent = cyan" once) or roles ARE the single source (leaning: roles-as-roots;
   a palette tier is optional indirection).

## Relationship to other quests
Supersedes/absorbs the deferred glk_styles style.toml selectors (noted in
SQ-0331) and the SQ-0309 "registry + single-level fallback" direction. Depends on
the shipped SQ-0325/0328/0329/0330/0331/0319 branch.
