# Styling redesign: role palette + full Glk namespace (SQ-0309)

**Status:** design spec (decided in brainstorm 2026-07-14; the 11 standard Glk
styles fully incorporated 2026-07-19 — see §3/§3a; per-game-vs-story precedence
decided 2026-07-19 — see §5). Implement AFTER the current Glk-borders/styling
branch merges — it touches the same styling code.

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

**Terminology (fixed):** **panels** are the frames *we* draw — story pane, map,
verb menu, debug inspector, dialogs/overlays, hints, tidy, glyph picker, etc.
**windows** are the surfaces the *story/VM* generates — Glk buffer/grid/graphics
windows, the v4+ upper window, multi-window layouts. Panels are styled by the
roles / elements / `panel.*` selectors; windows by the `glk.*` namespace and the
game-stylehint layers. The two never share a selector: panel chrome is host UI
(never honors game colours); window chrome can (§5).

### 1. Roles — the ~7 roots a theme actually sets
`text · chrome · line · accent · muted · alert · heading`
Each carries fg / bg / modifiers. (User chose "balanced".)

| Role | meaning |
|---|---|
| text | body ink on the page |
| chrome | ink on a UI surface (bars/panels/upper window) |
| line | lines / frames / rules / dividers |
| accent | highlight (links, selection, current, badges) |
| muted | dim / secondary (labels, meta, suggestions) |
| alert | warning / error |
| heading | emphasized titles / headers |

### 2. App elements — derivations of roles (`parent + delta`)
Non-map, non-debug app chrome/text. Map-domain selectors live in §4 `map.*` and
debug-window selectors in §4b `debug.*` — NOT here — so each visual domain is
declared once (no `[elements]`/`[map]` overlap).

| Element(s) | parent | delta |
|---|---|---|
| transcript | text | — |
| status_bar, help_bar, upper_window, story_info | chrome | — |
| status_header, story_title | heading | on chrome |
| input_line, suggestion_line, scrollbar | line | (role/side deltas) |
| scrollbar_track | muted | — (the dim channel the thumb runs in; SQ-0782) |
| transcript_location | accent | — |
| story_badge | accent | reverse |
| hyperlink | accent | underline |
| story_info_label, suggestion, transcript_meta, transcript_system | muted | — |
| transcript_warning | alert | — |
| transcript_crash | alert | bold |

### 2a. `panel.*` — one border for every panel, with focus states
Every **panel** frame we draw shares ONE border definition instead of a
per-panel `*_border` selector, so panels are uniform by construction. Two focus
states (the app already tracks pane focus for Tab navigation and picks between
them):

| selector | parent | delta | used when |
|---|---|---|---|
| `panel.background` | — (transparent) | — | the panel interior fill; transparent by default so panels show the terminal background, set a `bg` to give panels a solid surface |
| `panel.border` | line | `style = "single"` | panel is unfocused (inactive) |
| `panel.border:active` | line | `style = "single"`, bold | panel has focus (active) |

`:active` = `line + bold` reproduces today's `focused_border` (Cyan+BOLD)
exactly; retheme it to `parent = "accent"` for a colour-shift focus highlight.
Both border rows carry the full border grammar (`style`, per-side `style_<side>`,
`header`). `panel.background` is the standard body fill for every panel. Every app
panel — story pane, map, verb menu, debug inspector, dialogs/overlays (modal ⇒
always `:active`), hints, tidy, glyph picker, room-info, file browser — resolves
its frame AND body from these shared selectors; there are no per-panel border or
background selectors, unless a panel deliberately overrides the body (the map's
canvas does — `map.background`, §4).

**Separate from windows (§Terminology).** The story/VM **window** borders — the
upper-window frame (`upper_window_border`, which may adopt the game's page colours
per SQ-0267), Glk multi-window separators/borders (SQ-0328/0341), and any
game-drawn border — are NOT `panel.*`. They live in the window/`glk.*` domain and
honor game colours; `panel.*` is host chrome only and never does.

### 2b. `panel.*` header chrome — title and tabs (shared)
A panel's top strip may carry a **title** or **one or more tabs**, inset into the
top border between two terminators, e.g. `╭─┤ Disasm │ Stack ├────╮`. These are
shared `panel.*` selectors too, so every panel's header looks identical and the
debug inspector's window tabs and the map's layer tabs render through the same
styling (the old `debug.tab`/`map.map_layer_tab` style selectors are removed):

| selector | kind | default | meaning |
|---|---|---|---|
| `panel.title` | style | heading on chrome | a plain panel title inset in the top border |
| `panel.tab` | style | muted | an inactive / unselected tab |
| `panel.tab:active` | style | accent + bold | the active / selected tab |
| `panel.tab_divider` | glyph + style | glyph `│`, line | separator drawn between adjacent tabs |
| `panel.terminator_left` | glyph + style | matches border `style` (`┤`), line | left cap where the title/tab strip meets the frame |
| `panel.terminator_right` | glyph + style | matches border `style` (`├`), line | right cap of the strip |

Terminator glyphs default to the panel's border `style` so they stay consistent
across single/double/rounded frames (`┤`/`├`, `╡`/`╞`, …); override with an
explicit `glyph`. `panel.title` is the generic default — a panel that needs a
special title (e.g. `story_title`, styled as the game name) still overrides it.

**Pruning (two kinds):**
- *Auto-collapse:* any element whose default already equals its role becomes a
  pure derivation (no independent declaration; zero visual change). Audit the
  default theme; likely most elements.
- *Retire:* shortlist to remove entirely (bring to user to veto) — candidates:
  `meta_marker` / `warning_marker` (as separate from the text they mark),
  `story_info_cover`, `transcript_system` (merge into `transcript_meta`).

**Text sections vs surface sections.** The schema splits along a clean line.
**Text sections** — `[elements]`, `[glk.buffer]`, `[glk.grid]`, `[map]`,
`[debug]` — only adjust a *foreground* colour + emphasis for content drawn on an
existing surface. **Surface sections** — `[panel]`, `[dialog]`, `[tooltip]` —
describe a whole surface: a background fill, an optional border/frame, and the
text drawn on it. The panel is one such surface (§2a/§2b); dialogs (§2c) and
tooltips (§2d) are two more, each with its own background + frame instead of
borrowing the panel's.

### 2c. `[dialog]` — the modal surface (its own frame)
A dialog/overlay is a **surface**, not a plain panel: it has its own background,
its own frame, a title, buttons, and a drop-shadow. These used to be flat
`[elements]` selectors (`dialog`, `dialog_title`, `dialog_button`,
`dialog_button_active`, `dialog_shadow`); they are now a dedicated `[dialog]`
section with **bare** keys (the internal full lookup name in parens):

| key | internal name | parent | delta |
|---|---|---|---|
| `background` | `dialog.background` | chrome | body fill |
| `border` | `dialog.border` | line | **NEW:** the dialog's own frame — `style = "single"`, bold; colour *and* style both come from here |
| `title` | `dialog.title` | accent | — |
| `button` | `dialog.button` | chrome | reversed |
| `button:active` | `dialog.button:active` | accent | reversed |
| `shadow` | `dialog.shadow` | muted | bg dark-gray |

The dialog frame used to borrow `panel.border:active`; it now has its **own**
`dialog.border` (a full border grammar — colour + `style` come from it), so a
theme can frame dialogs differently from panels.

### 2d. `[tooltip]` — the shared hover-tooltip surface
Hover tooltips (e.g. the debug inspector's opcode tooltip, formerly the
`debug.tooltip` selector) are a shared **surface** section, so every tooltip in
the app looks the same:

| key | internal name | parent | default style |
|---|---|---|---|
| `background` | `tooltip.background` | chrome | body fill |
| `border` | `tooltip.border` | line | `style = "none"` (borderless; set a style to frame the tooltip) |

Keys under `[dialog]` / `[tooltip]` are written **without** the dotted prefix
(e.g. `title = { parent = "accent" }`), exactly like the `[panel]` / `[map]`
keys — the dotted form (`dialog.title`, `tooltip.border`) is only the internal
lookup name.

### 3. `glk.*` — the 11 standard Glk styles (first-class, defaults to roles)
The Glk spec defines exactly **11 styles**, numbered 0–10, which the gvm already
carries as `GlkStyle` (`glk.rs`). Every style × window type is an addressable
selector — `glk.buffer.<style>` (text-buffer windows) and `glk.grid.<style>`
(text-grid / status windows) — 22 slots total. Each **defaults to a role
derivation** so authors set nothing and get coherence, but can override any one
slot for full Glk fidelity ("full support for glk" — user).

**Slot model (decided):** a slot is a full style — `fg / bg + modifiers`
(bold / italic / underline / reversed) — NOT colour-only. The standard Glk
styles are typographic (Emphasized is slanted, Header/Subheader/Alert are
heavier), so colour-only slots can't represent them; a slot carries a modifier
delta layered on its parent role. This widens today's `GlkStyleColour {fg,bg}`
to the shared `Style` and bumps the Glk host-snapshot format (pre-release: no
shim). Terminal note: Preformatted's "monospace" and any proportional/size hints
are no-ops in a TUI (cells are already fixed-width) — they resolve to colour
only; the terminal-meaningful modifiers are bold / italic / underline / reversed.

**The 11 canonical defaults** — `parent role + standard delta` (decided
2026-07-19). Buffer base = `text`; grid base = `chrome`. Same style number, same
delta on both rows; only the base role differs.

| # | Glk style | parent role | standard delta |
|---|---|---|---|
| 0 | Normal | text / chrome | — |
| 1 | Emphasized | text / chrome | italic |
| 2 | Preformatted | text / chrome | (mono → colour-only in TUI) |
| 3 | Header | heading | bold |
| 4 | Subheader | heading | bold |
| 5 | Alert | alert | bold |
| 6 | Note | muted | italic |
| 7 | BlockQuote | muted | — |
| 8 | Input | accent | — |
| 9 | User1 | text / chrome | — |
| 10 | User2 | text / chrome | — |

`Normal` is definitionally its base role (buffer Normal ≡ text, grid Normal ≡
chrome), so a game that touches no styles renders byte-identical to a
role-only theme — preserving the SQ-0331 "all-Normal ⇒ no change" invariant.
User1/User2 have no Glk-defined appearance; they inherit the base role and exist
purely as override slots.

### 3a. Import mapping — how the standard styles land in the slots
The slots are the single sink for every style source, so import is one merge per
source into the same 22-slot table (most-specific source wins per channel):

- **garglk.ini** (SQ-0319 importer): `tcolor N` → `glk.buffer.<N>` fg/bg;
  `gcolor N` → `glk.grid.<N>` fg/bg (already implemented). Colour is the ONLY
  per-style signal garglk.ini carries. (CORRECTION 2026-07-19, verified against
  Gargoyle: garglk.ini's `stylehint` directive is a **global boolean** — `0`/`1`
  disables/enables honoring the game's style hints, already mapped to
  `honor_game_colours`. There is NO per-style `stylehint <wintype> <style> <hint>
  <value>` line in the .ini; bold/italic in Gargoyle come from separate font files
  (`monob`/`monoi`/`propb`/`propi`), which are terminal no-ops. The earlier draft of
  this section conflated the .ini directive with the Glk *API* below.)
- **Runtime `glk_stylehint_set(wintype, style, hint, value)`** (the Glk API the
  game calls; gvm records `Weight`(4)/`Oblique`(5) per `(wintype, style)`): map
  `weight 1`→bold / `weight -1`→(dim, no terminal equivalent → ignore),
  `oblique 1`→italic (underline/reverse are colour-hint driven). Applied per-window
  as the SQ-0328 game-stylehint layer, honor-gated. gvm already stores the values;
  wiring them to render modifiers lands with §5's runtime chain (plan Wave 3), NOT
  as a garglk import. This is the real per-style modifier source.
- **`style.toml`** author overrides: `[glk.buffer]` / `[glk.grid]` tables set any
  slot's fg/bg/modifiers directly (see schema below).

All three resolve through §5's chain into the flat theme map; nothing else in the
app reads a Glk style outside these slots.

### 3b. Window background colour = the Normal slot's bg
A text window's background — the fill behind the text, including unwritten cells —
is *by definition* its **Normal style's bg** for that window type (this is how the
renderer already works: the content-fill style's bg paints empty cells,
`render/upper_window.rs` / `transcript.rs`). So window background needs no separate
knob; it falls out of the model:
- **Buffer window bg** = `glk.buffer.normal.bg` → defaults to `text.bg`.
- **Grid / status window bg** = `glk.grid.normal.bg` → defaults to `chrome.bg`.

Set the base **role** bg (`text.bg` / `chrome.bg`) to recolour the page/UI-surface
for every window of that type; override `glk.<wintype>.normal.bg` to recolour just
one window type without touching text elsewhere.

**Panels** (§Terminology) are not Glk windows, so their body background is the
standard `panel.background` (§2a) — this covers the debug inspector, verb menu,
hints, tidy, glyph picker, etc. The **map** pane deliberately overrides it with its
own canvas fill, `map.background` (§4). Between the Glk-window rule above and these
two panel rules, every pane's background is addressable.

**Per-instance targeting is intentionally not offered.** Glk windows are created
by the game at runtime with no stable, user-addressable identity, so a user theme
specifies backgrounds by window *type* (buffer vs grid) or Glk style — never
"window #3". A game that sets a specific window's background does so via its own
`glk_stylehint` on that window's Normal bg (or a page scheme); that rides the
per-window game-stylehint layer (§5, SQ-0328) and wins or loses per
`honor_game_colours` and the per-game precedence rule.

**Graphics windows are a separate path.** `glk_window_set_background_color` /
`glk_window_fill_rect` set a graphics-canvas fill, not a text style — resolved in
the graphics/`map.*`-adjacent handling (gated by `honor_game_colours`), NOT
through the `glk.*` text slots.

### 4. `map.*` — OPEN DESIGN AREA (user flagged "other options")
The map is a distinct visual domain (room fills vs borders, edge colours,
current/selected/visited distinction, per-layer colour coding, distortion
markers) that does not map cleanly onto the text-oriented roles. Give it its own
`map.*` sub-palette. Options to decide in a follow-up:
- **(a) Own roots:** a small independent map palette (room, room-current,
  room-selected, connector, layer-cycle, map-border) — most control, more knobs.
- **(b) Reference shared roles:** map elements derive from the shared roles
  (room-current → accent, connector → line/muted) with map-specific deltas —
  fewer knobs, coupled to the app palette.
- **(c) Hybrid:** map has its own *tokens* (fills/edges/layer cycle) but current
  /selection reference the shared `accent` so highlight colour stays consistent.
Recommendation: (c). Decide during implementation; not blocking the role palette.

**`map.*` is the sole owner of every map-domain selector** (no `[elements]`
overlap): colours — `background`, `room`, `room_current`, `room_selected`,
`connector`, `connector_distorted`, `connector_portal`, `shared_path`,
`loc_indicator`, `layer_cycle`; and glyph-set presets — `box_style`, `arrow_set`,
`portal_icons`, `path_style`, `portal_path_style`. Individual glyph overrides
attach to the relevant selector (a `glyphs` sub-map on `map.room` for box slots,
on `map.connector` for arrows), not a separate table (see §2/glyph decision).
Under hybrid (c): `room_current`/`room_selected` → `parent = "accent"`; own tokens
for fills/edges/the per-layer `layer_cycle`. The map pane **frame** uses the
unified `panel.border` (§2a), and its **layer tabs** render through the shared
`panel.tab` / `panel.tab:active` (§2b) — so there is no `map_layer_tab` selector.

**`[symbols]` is merged into `[map]` (decided 2026-07-19).** The old top-level
`[symbols]` section held box/arrow/portal/path presets that are all map glyphs, so
they become `map.*` keys. There is **no glyph-override table** (`[symbols.overrides]`
is gone): a per-glyph override attaches to the selector it decorates as a `glyph`
(single) or `glyphs` (named sub-map) attribute — box slots on `map.room`, arrows on
`map.connector`, the meta/warning gutter marks on `transcript_meta`/`transcript_warning`
(§2), terminators/dividers on `panel.*` (§2b), the confidence marks on the `debug.*`
tiers (§4b). Net: every glyph lives on the selector it belongs to; no standalone
`[symbols]` section and no override table.

**New: separate up/down/in/out path style.** Cardinal (N/S/E/W) connectors keep
`path_style`; vertical/portal connectors (up/down/in/out) get their own
`portal_path_style` (same `light`/`heavy`/`dotted` presets), so a theme can render
portal runs distinctly (e.g. dotted) from cardinal ones.

**Map panel background (answering the review q):** the map pane has no Glk
Normal slot, so it gets an explicit **`map.background`** token — the interior fill
behind rooms/edges, defaulting to the page/terminal `background`. (Frame =
`panel.border`; interior = `map.background`.)

### 4b. `debug.*` — the /debug inspector windows (own namespace)
The debug inspector (disassembly / registers / stack, SQ-0420) is a third visual
domain with an existing selector family (`debug_pane`, `debug_title`,
`debug_disasm_pc`, `debug_tab`/`_active`, `debug_exec_mark`) and
the pending SQ-0428 confidence tiers. Give it a `debug.*` sub-palette paralleling
`map.*`, deriving from the shared roles so it stays coherent with the app chrome:

| debug selector | kind | default | meaning |
|---|---|---|---|
| pc | style | accent + reverse | the line at the current PC |
| disasm_executed *(SQ-0428)* | style + glyph | accent, glyph `|` | proven code (a PC that ran) — styles the line AND its gutter mark |
| disasm_rd *(SQ-0428)* | style + glyph | text, glyph ` ` | reached by recursive descent from a call |
| disasm_soft *(SQ-0428)* | style + glyph | muted, glyph ` ` | linear-scan guess (soft boundary) |
| disasm_data *(SQ-0428)* | style + glyph | muted + italic, glyph ` ` | non-code / data bytes |

Each confidence tier carries **both** a line style and a gutter **`glyph`** (the
mark drawn in the gutter column for lines at that tier), so the colour *and* the
mark character are themeable. This subsumes the old `exec_mark` — the executed
tier's glyph (default `|`) IS the executed gutter mark; there is no separate
`exec_mark` selector. Non-executed tiers default to a blank glyph (no mark); give
one a glyph to mark it (e.g. `?` on `disasm_soft`).

The debug panel uses the **standard panel chrome** — body from `panel.background`,
frame from `panel.border` / `panel.border:active` (§2a), title and window tabs
from `panel.title` / `panel.tab` / `panel.tab:active` (§2b), and its opcode hover
tooltip is the shared `[tooltip]` surface (§2d), not a `debug.*` selector. So
`debug.*` holds
ONLY genuinely debug-specific content selectors (the disassembly rendering); there
is no `debug.panel`, `debug.border`, `debug.title`, `debug.tab`, or `debug.tooltip`. Debug panels
are host UI, never game surfaces
— the §5 game-stylehint layers do NOT apply; they resolve straight from
`debug.*` → role → terminal. This folds the SQ-0428 confidence-colouring work into
the registry (one row per tier + a render apply site) instead of a separate
ad-hoc selector set.

### 5. Resolution chain (SQ-0331 base + per-game precedence, decided 2026-07-19)
Per channel, most-specific wins. The full order, highest first:

`user per-game override (explicit) → garglk per-stream override → game stylehint
(per-window, SQ-0328 hybrid) → glk.* slot (= glk defaults + user global theme +
shipped garglk.ini, see registry) → its role → terminal.`

Gate `honor_game_colours`: on → normal; off → drop the two game layers (garglk
per-stream + game stylehint), i.e. `slot.or(role)`. garglk.ini imports into the
`glk.*` slots (SQ-0319).

**Per-game precedence (new).** An *explicitly-set* user per-game slot sits ABOVE
the two story layers — it wins over the game's live stylehints and any garglk
per-stream override, effectively a **per-slot `honor_game_colours = off`** for
just that slot. This is per-slot and provenance-gated: only slots the user
actually set for this game are lifted; every unset slot falls through to the game
exactly as before. The **global** user theme is NOT lifted — it stays inside the
`glk.*` slot layer and still defers to the game when `honor_game_colours` is on
(a general preference yields to an opinionated author; a per-game override does
not). Rationale: specificity + explicitness — the most specific *and* deliberate
user signal wins.

Provenance requirement: the resolver must distinguish "user set this per-game
slot" from "inherited/default" so the lift applies only to explicit per-game
values. The registry already carries per-slot provenance (see below).

### 5a. Storage of overrides
- **Global user theme:** `<user_dir>/<style-file>.toml` (the `style` pointer in
  `config.toml`) — unchanged.
- **Per-game overrides:** the **game-specific save directory** —
  `game_dir = <data_base>/<story-key>/`, default `<user_dir>/saves/<story-key>/`,
  the same folder that holds `map.txt` and the game's saves (SQ-0284 layout).
  Per-game style slots live in `<game_dir>/style.toml`; the per-game
  `honor_game_colours` flag in `<game_dir>/config.toml`. Both already exist today
  (`styles::per_game_style_path` / `read_per_game_honor`); the redesign keeps this
  location and extends the file to the new `[roles]`/`[elements]`/`[glk.*]` schema.
- **Shipped garglk.ini:** beside the story file (SQ-0319 discovery) — a *story*
  asset, not a user override; imported into the slots, below the per-game layer.

## Registry (the implementation core)
One declarative table; each row: `name`, template section, kind (style /
border+glyphs / placement), parent (role or another selector), optional delta,
default. A single resolve pass computes the flat theme map. Static build order
(later overrides earlier): roles → element derivations → glk defaults → user
global decls → shipped garglk.ini → **user per-game overlay** (moved LAST so a
story-shipped garglk.ini cannot clobber an explicit per-game setting — §5). The
resolver records per-slot **provenance** (which layer supplied the value) so the
runtime chain can lift explicit per-game slots above the game layers. The table
drives: parsing, resolution with parent fallback, template-section grouping (the
commented seed template + `style.example.toml`), and `/dump`/export round-trip.
This deletes the 5 duplicated enumerations. (There is no interactive editor to
drive — see Delivery.)

## TOML schema (breaks the old one — pre-release, no shim)
```
[roles]           # the ~7 roots
text   = { fg = "...", bg = "..." }
accent = { fg = "..." }
...
[elements]        # overrides only; unset = role derivation
hyperlink = { parent = "accent", underline = true }
[glk.buffer]      # overrides only; unset slot = its canonical §3 default
header    = { bold = true }                 # modifier delta on the heading role
alert     = { fg = "...", bold = true }     # colour + modifier
[glk.grid]
input     = { parent = "text" }             # re-root a slot off its default role
[panel]           # SURFACE: shared panel chrome — background, border(+:active), title, tab(+:active), dividers/terminators
[dialog]          # SURFACE: modal — background, its OWN border, title, button(+:active), shadow
[tooltip]         # SURFACE: shared hover tooltip — background + optional border (borderless by default)
[map]             # map colours + glyph-set presets (old [symbols] merged in); per-glyph overrides via a `glyphs` sub-map on the selector
[debug]           # disasm-only: pc, confidence tiers (each with a gutter glyph)
[statusbar] / [[transcript.rule]]  # carried over
```

## Full `style.toml` example (for review)
A complete theme in the new schema, expressing today's default (dark) look. It is
written out **in full** for review — but almost every line equals its registry
default, so a real theme keeps only the lines it changes. Value grammar is
unchanged from today (`style.example.toml`): a named colour, `palette:N`,
`#rrggbb`, a 256-index, or `background`/`foreground`. Modifier keys:
`bold`/`italic`/`underline`/`dim`/`reversed`. Border selectors also take
`style`/`style_<side>`/`header`; `dialog` takes `shadow`.

```toml
scheme = "tomorrow-night"     # optional base: built-in name or a Ghostty theme path; omit for terminal colours

# ── Roles: the 7 roots everything derives from ───────────────────────────────
[roles]
text    = { fg = "white",     bg = "background" }  # body ink on the page
chrome  = { fg = "white",     bg = "black" }       # ink on a UI surface (bars/panels/upper window)
line    = { fg = "cyan" }                          # lines / frames / rules / dividers
accent  = { fg = "cyan" }                          # highlight: links, selection, current room, badges
muted   = { fg = "dark-gray" }                     # dim / secondary
alert   = { fg = "yellow" }                        # warning / error
heading = { fg = "white",     bold = true }        # titles / headers

# ── Panel border: ONE frame for every panel we draw, with focus states. ──────
# ── (Story/VM windows have their own borders — see [glk.*], not here.) ────────
[panel]
background       = { parent = "chrome" }                               # standard panel body fill (all panels)
border           = { parent = "line", style = "single" }               # unfocused frame
"border:active"  = { parent = "line", style = "single", bold = true }  # focused frame (today's cyan+bold)
title            = { parent = "heading" }                              # a plain panel title
tab              = { parent = "muted" }                                # inactive tab
"tab:active"     = { parent = "accent", bold = true }                  # active/selected tab
tab_divider      = { glyph = "│", parent = "line" }                    # between tabs
terminator_left  = { parent = "line" }          # cap where the strip meets the frame; glyph defaults to border style (┤)
terminator_right = { parent = "line" }          # …├ ; set glyph = "…" to override

# ── Dialog surface (§2c): a background + its OWN frame + the text on it. ───────
# ── A SURFACE section, separate from [panel]: the modal frame is dialog.border ─
# ── (not borrowed from panel.border:active). ─────────────────────────────────
[dialog]
background       = { parent = "chrome", bg = "black" }                 # the modal body fill
border           = { parent = "line", style = "single", bold = true }  # dialog's OWN frame (colour + style)
title            = { parent = "accent" }                               # the dialog title
button           = { parent = "chrome", reversed = true }              # a dialog button
"button:active"  = { parent = "accent", reversed = true }              # the focused/active button
shadow           = { parent = "muted", bg = "dark-gray" }              # drop-shadow behind the frame

# ── Tooltip surface (§2d): a background + optional frame + the text on it. ─────
# ── Shared by every hover tooltip (e.g. the debug inspector's opcode hover). ──
[tooltip]
background       = { parent = "chrome" }                               # the tooltip body fill
border           = { parent = "line", style = "none" }                 # borderless by default; set a style to frame it

# ── Elements: parent + delta. Shown in full; every line == its default, so a ──
# ── minimal theme may delete this whole section and look identical. ───────────
[elements]
transcript           = { parent = "text" }
status_bar           = { parent = "chrome", reversed = true }
help_bar             = { parent = "chrome", reversed = true }
upper_window         = { parent = "chrome" }
story_info           = { parent = "chrome" }
status_header        = { parent = "heading", bg = "black" }
story_title          = { parent = "heading" }
input_line           = { parent = "line" }          # add style = "single" to box it
suggestion_line      = { parent = "line" }          # add style = "single" to box the popup
scrollbar            = { parent = "line" }          # the thumb — drawn as a background fill, not a glyph
scrollbar_track      = { parent = "muted" }         # the channel behind it, likewise a fill
transcript_location  = { parent = "accent" }
story_badge          = { parent = "accent", reversed = true }
hyperlink            = { parent = "accent", underline = true }
# NB: panel frames (story/map/verb/debug) come from [panel] (§2a); the
# upper-window frame is a WINDOW border (game domain), not set here; map/debug
# selectors live in [map]/[debug]; dialog/tooltip surfaces live in [dialog]/[tooltip].
story_info_label     = { parent = "muted" }
suggestion           = { parent = "muted" }
transcript_meta      = { parent = "muted", glyph = "▏" }   # glyph = the meta gutter mark (was symbols "gutter.meta")
transcript_warning   = { parent = "alert", glyph = "!" }   # glyph = the warning gutter mark
transcript_crash     = { parent = "alert", bold = true }

# ── The 11 Glk styles × buffer/grid. Overrides only; unset = §3 canonical ────
# ── default. Buffer base = text, grid base = chrome. Window background of a ───
# ── window type = its `normal.bg` here (§3b). ────────────────────────────────
[glk.buffer]
normal       = { parent = "text" }
emphasized   = { parent = "text",    italic = true }
preformatted = { parent = "text" }                 # mono is a TUI no-op — colour only
header       = { parent = "heading", bold = true }
subheader    = { parent = "heading", bold = true }
alert        = { parent = "alert",   bold = true }
note         = { parent = "muted",   italic = true }
blockquote   = { parent = "muted" }
input        = { parent = "accent" }               # echoed player input
user1        = { parent = "text" }
user2        = { parent = "text" }

[glk.grid]
normal       = { parent = "chrome" }               # status/upper-window background lives here
emphasized   = { parent = "chrome",  italic = true }
preformatted = { parent = "chrome" }
header       = { parent = "heading", bold = true }
subheader    = { parent = "heading", bold = true }
alert        = { parent = "alert",   bold = true }
note         = { parent = "muted" }
blockquote   = { parent = "muted" }
input        = { parent = "accent" }
user1        = { parent = "chrome" }
user2        = { parent = "chrome" }

# ── Map palette — hybrid (c): own tokens for fills/edges, but current/selected ─
# ── reference the shared `accent` so highlight colour stays consistent. ───────
# ── (map.* model still an open question §4 — this shows the recommended shape.) ─
[map]
background            = { bg = "background" }        # overrides panel.background for the map canvas (frame = panel.border)
room                 = { fg = "white" }             # box glyphs: add glyphs = { tl = "+", h = "-" } to override individual slots
room_current         = { parent = "accent" }
room_selected        = { parent = "accent", reversed = true }
connector            = { fg = "cyan" }              # arrow glyphs: add glyphs = { north = "^", up = "↑" } to override
connector_distorted  = { fg = "magenta" }
connector_portal     = { fg = "cyan" }
shared_path          = { fg = "light-cyan" }
layer_cycle          = ["cyan", "green", "magenta", "yellow", "blue"]  # per-layer edge colour cycle
loc_indicator        = { parent = "muted" }
# ── Map-wide glyph SET presets (the old [symbols] section, merged in). Per-glyph─
# ── overrides attach to the selectors above via a `glyphs` sub-map, not a table.─
box_style            = "rounded"    # room boxes: rounded | thick | double | solid | super-thick | ascii | borderless
arrow_set            = "filled"     # cardinal connector arrows: filled | line | nerdfont | nf-bold | nf-box | nf-circle | nf-outline
portal_icons         = "ascii"      # up/down/in/out endpoint icons: ascii | nerdfont | nerdfont-stairs
path_style           = "light"      # cardinal (N/S/E/W) connector line: light | heavy | dotted
portal_path_style    = "dotted"     # up/down/in/out connector line — styled separately from cardinal paths
# NB: the map's layer tabs render through [panel] tab/tab:active (§2b), not here.

# ── Debug inspector (§4b): host UI. Uses standard [panel] chrome for body / ───
# ── frame / tabs; debug.* holds ONLY the disassembly-specific selectors. ──────
# ── Confidence tiers are the SQ-0428 disasm colouring folded into the registry.─
[debug]
pc                  = { parent = "accent", reversed = true }   # line at the current PC
# (opcode hover tooltip is the shared [tooltip] surface, not a debug.* selector)
# Confidence tiers (SQ-0428): each styles the disasm line AND sets its gutter mark
# glyph. disasm_executed's glyph IS the executed gutter mark (was exec_mark).
disasm_executed     = { parent = "accent", glyph = "|" }
disasm_rd           = { parent = "text",   glyph = " " }
disasm_soft         = { parent = "muted",  glyph = " " }       # e.g. glyph = "?" to mark guesses
disasm_data         = { parent = "muted",  glyph = " ", italic = true }

# ── Story-line styling rules ─────────────────────────────────────────────────
# Each rule recolours whole transcript lines that match a regex. Rules are tried
# in order; the first match wins, ahead of the built-in location/system rules.
# `match` is a regex (`(?i)` = case-insensitive, `\b` = word boundary); the other
# keys are the same style keys as any selector (fg/bg/bold/italic/…).
[[transcript.rule]]
match = "^>.*"                 # your echoed command lines → magenta bold
fg = "magenta"
bold = true

[[transcript.rule]]
match = "(?i)\\bgrue\\b"       # any line mentioning a "grue" → red (flavour example)
fg = "red"

# ── Status bar (carried over). Omit [statusbar] for the built-in default. ─────
# Placeholders: {location} {score} {moves} {time} {turns} {title} {filter}
[statusbar]
border = "none"

[[statusbar.segment]]
text  = "{location}"
align = "left"
parent = "accent"              # segments may reference a role or set fg/bg directly
bold  = true

[[statusbar.segment]]
text  = "Score: {score}  Moves: {moves}"
align = "right"

[[statusbar.segment]]
text  = "{time}"
align = "right"

# (The old [symbols] section is gone — map glyph presets moved into [map]; the
#  transcript gutter marks moved onto transcript_meta / transcript_warning as
#  their `glyph`; panel/debug glyphs live in [panel]/[debug].)
```

Notes for review:
- `parent` may name a role or another selector; omit it and a slot/element uses
  its registry-default parent. A delta is any fg/bg/modifier set alongside.
- `[panel]` is the shared chrome for every panel we draw: body (`background`),
  frame (`border` / `border:active`, §2a), and header chrome (`title`, `tab` /
  `tab:active`, `tab_divider`, `terminator_left` / `terminator_right`, §2b). No
  panel sets its own body, frame, or tab style — story/map/verb/debug all
  resolve here, so debug window-tabs and map layer-tabs look identical. The only
  per-panel override in the example is `map.background` (the canvas). `[debug]`
  therefore holds only disassembly-specific selectors. Dialogs and tooltips are
  separate **surface** sections (`[dialog]` §2c, `[tooltip]` §2d) with their own
  background + frame, not `[panel]` chrome. Story/VM **window** borders
  are separate (`glk.*`, game domain).
- `tab_divider` / `terminator_*` carry a `glyph`; the terminator glyph defaults to
  the panel border `style` (┤/├ for single, ╡/╞ for double) so caps match the frame.
- `[symbols]` is gone: map glyph-set presets (`box_style`/`arrow_set`/`portal_icons`/
  `path_style`/new `portal_path_style`) live in `[map]`; there is NO override table —
  per-glyph overrides are a `glyph`/`glyphs` attribute on the selector they decorate
  (gutter marks on `transcript_meta`/`transcript_warning`, box/arrow slots on
  `map.room`/`map.connector`, etc.).
- `[statusbar.segment]` gains an optional `parent` so a segment can ride a role
  instead of restating a colour (new; the rest of the statusbar block is as-is).
- `map.layer_cycle` is an ordered colour list (the only list-valued key); every
  other selector is an inline table. Its shape is provisional pending §4.
- The example shows values resolved for readability. The **shipped/seeded template
  is this same content fully commented out** (see Delivery), so the registry
  defaults apply until a user uncomments a line.

## Delivery & workflow (no interactive editor)
The interactive styling UI is **removed** and replaced by an editable, fully
commented template plus live reload (decided 2026-07-19):

- **Removed:** the live **style editor** (`render/style_editor.rs`,
  `style_actions.rs`, `style_mru.rs`, `open-style-editor`), its **glyph picker**
  (`render/glyph_picker.rs`, `glyph_actions.rs`, all `GlyphPicker*` actions +
  `StyleOpenGlyphPicker`), and the **symbol gallery** (`render/gallery.rs`,
  `open-gallery`, `OpenGallery`, the gallery input sub-mode, its `f` binding).
  KEPT: the story **cover-art gallery** (`cover_gallery.rs`, `g`, SQ-0374) — it is
  unrelated to styling.
- **Auto-seeded template:** on startup, if the user has no `style.toml`, the app
  writes a **fully commented-out** template (every selector present, commented,
  with a short explanatory comment per section — roles, panels, glk.*, map, debug,
  transcript rules, statusbar). It **never overwrites** an existing file. The
  registry generates it, so it is always complete and in sync with the selectors.
- **Editing loop:** the user uncomments/edits lines and runs the existing
  **`reload-style`** command (already present) to apply changes live; a syntax
  error keeps the current look and warns. `watch_style = true` still auto-reloads
  on save.
- The repo's `style.example.toml` is regenerated from the same registry so it
  matches the seeded template.

## Migration & compatibility
Pre-release: the style.toml schema breaks; the registry regenerates the seeded
template + `style.example.toml`; README notes old files aren't migrated. No
back-compat decoder. An existing (old-schema) `style.toml` is left untouched by
the auto-seed (no overwrite); it simply resolves with warnings until the user
regenerates — acceptable pre-release.

## Open questions (for implementation)
1. Map role model — options (a)/(b)/(c) above (user flagged; lean hybrid (c)).
2. Final retire list (auto-collapse is safe; retirals need a veto).
3. Whether a separate raw-colour "palette" tier sits under the roles (change
   "accent = cyan" once) or roles ARE the single source (leaning: roles-as-roots;
   a palette tier is optional indirection).

*Resolved 2026-07-19:* Glk slot model = fg/bg + modifiers; Input → accent;
Note/BlockQuote → muted; grid base role → chrome; the 11 canonical defaults and
the garglk.ini/runtime import mapping (§3/§3a). Per-game precedence lifts explicit
per-game slots above story values (§5); per-game overrides stored in the
game-save dir (§5a). Unified `panel.*` chrome — body/frame(+focus)/title/tabs/
terminators (§2a/§2b) — shared by all panels; `panel.*` = us, `glk.*` = story/VM
windows. `map.*` owns map colours + glyph-set presets (`[symbols]` merged in) and
gains a separate `portal_path_style`; NO glyph-override table — per-glyph overrides
are a `glyph`/`glyphs` attribute on each selector. `debug.*` = disasm-only;
`exec_mark` folded into
`disasm_executed` (tiers carry a themeable gutter glyph). Interactive editor +
glyph picker + symbol gallery removed; a fully commented template is auto-seeded
if absent (never overwritten) and applied via the existing `reload-style`.

## Relationship to other quests
Supersedes/absorbs the deferred glk_styles style.toml selectors (noted in
SQ-0331) and the SQ-0309 "registry + single-level fallback" direction. Depends on
the shipped SQ-0325/0328/0329/0330/0331/0319 branch.
