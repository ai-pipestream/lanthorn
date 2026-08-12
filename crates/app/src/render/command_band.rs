//! The command band: a persistent bottom dock that composes a command from
//! progressive columns (Journey's command bar, minus the party column).
//!
//! Replaces the old left-edge verb/noun/prep token palette (SQ-0664). The
//! differences that matter:
//!
//! * Every verb declares an **arity** (see [`Arity`]), so the band knows which
//!   column comes next and can dim the ones that are not reachable yet.
//! * The object columns are **live**: they are refreshed from the engine's
//!   object tree every frame (`loop_tick::refresh_command_band_objects`), not
//!   scraped from the transcript and snapshotted at open.
//! * It is **not a modal**. The story prompt stays live, paste keeps working,
//!   graphical v6 stays on the pixel path, and only clicks inside the band's
//!   own rect are taken from the game.
//! * A column pick composes directly onto the REAL story input line
//!   (`state.input`), not a band-local phrase row — retired 2026-08-05
//!   (SQ-0667) along with the band's own frame and its VERB column header;
//!   see the module doc in `crate::state` (`CommandBandState`) for how the
//!   mirroring works and `docs/design/2026-08-05-verb-panel-redesign.md` for
//!   why. Nothing auto-submits: composing still never fires a turn by
//!   itself — Enter on the real prompt is the confirm, same as anything
//!   typed by hand. The ONE exception is the quick row: a quick pick fires
//!   at once, no Enter (the immediate-fire half of the same amendment).
//! * **Typing always wins** (SQ-0676, 2026-08-05): the band never owns text
//!   keys. It READS the prompt — the word under construction there highlights
//!   the nearest match in whatever column is current.
//! * **A current column, moved by Tab (SQ-0677, 2026-08-05 — supersedes
//!   SQ-0676's arrow scheme)**: `Tab`/`Shift-Tab` step it across the
//!   reachable columns, `↑`/`↓` highlight a row within it, and `Tab` with a
//!   row highlighted (explicit or the typed nearest match) picks it and
//!   advances — the same gesture a click already was. `Enter` always submits
//!   the prompt as typed; it never picks. `←`/`→` are plain cursor movement on
//!   the edit line. The quick block (rose + flowing words, and the flat-row
//!   fallback) is mouse-click-only, with a hover highlight its one transient
//!   state. See `docs/design/2026-08-05-verb-panel-redesign.md`'s dated
//!   amendments for the full gesture table and why it changed twice in one
//!   day.
//!
//! ```text
//!  NW  N  NE │VERB     │WHAT — here │WHAT — carried│WITH…
//!   W  ·  E  │ look     │ window     │ brass key    │▸brass key
//!  SW  S  SE │▸unlock   │▸iron door  │ lantern      │ lantern
//!  up   down │
//!  in    out │
//!  look   inv│
//!  wait again│
//! ```
//!
//! (The `> unlock iron door with _` prompt line above this strip in the
//! mockup is the ordinary story input, drawn elsewhere — not part of this
//! module anymore.) The quick block now STACKS — rose on top, the
//! non-compass words flowing below it — rather than sitting beside the
//! columns (SQ-0677's other geometry change); see the "Quick-block layout"
//! section below. Single-cell `│` dividers separate the block from VERB and
//! every column from its neighbour, full band height.
//!
//! The caller (`main.rs`) sizes `area` from the animated `PanelSlide`
//! fraction, so `area` may be shorter than the band's target height while a
//! slide is in flight — everything here clips to `area`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::state::AppState;

// ── Grammar: the arity table ─────────────────────────────────────────────────

/// How many (and which) object slots a verb takes. Progressive disclosure needs
/// each verb to declare its shape; this is what decides which columns are
/// reachable after the verb is picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arity {
    /// `look`, `wait`, `n` — the phrase is complete with the verb alone.
    Solo,
    /// `take`, `open` — one object, required.
    Object,
    /// `search`, `push` — one object, but the verb alone is also valid.
    ObjectOpt,
    /// `unlock … with`, `put … in` — an object plus a prepositional second
    /// object. [`VerbEntry::prep`] carries the preposition.
    Pair,
}

impl Arity {
    /// Parse the config spelling of an arity (`"solo"`, `"object"`,
    /// `"object_opt"`/`"object?"`, `"pair"`). `None` for anything else, so a
    /// typo in `config.toml` is reported rather than silently reinterpreted.
    pub fn parse(s: &str) -> Option<Arity> {
        match s.trim().to_ascii_lowercase().as_str() {
            "solo" => Some(Arity::Solo),
            "object" => Some(Arity::Object),
            "object_opt" | "object?" | "objectopt" => Some(Arity::ObjectOpt),
            "pair" => Some(Arity::Pair),
            _ => None,
        }
    }
}

/// One verb the band offers, with the grammar it implies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerbEntry {
    pub word: String,
    pub arity: Arity,
    /// The preposition joining the two objects of an [`Arity::Pair`] verb
    /// (`unlock … **with** …`). Ignored for every other arity.
    pub prep: Option<String>,
}

impl VerbEntry {
    pub fn new(word: &str, arity: Arity, prep: Option<&str>) -> Self {
        VerbEntry { word: word.to_string(), arity, prep: prep.map(str::to_string) }
    }
}

/// The built-in verb table: `(word, arity, prep)`. Replaced wholesale by
/// `[command_band] verbs`, or extended by `extra_verbs`.
const BUILTIN_VERBS: &[(&str, Arity, Option<&str>)] = &[
    ("look", Arity::Solo, None),
    ("inventory", Arity::Solo, None),
    ("wait", Arity::Solo, None),
    ("again", Arity::Solo, None),
    ("north", Arity::Solo, None),
    ("south", Arity::Solo, None),
    ("east", Arity::Solo, None),
    ("west", Arity::Solo, None),
    ("up", Arity::Solo, None),
    ("down", Arity::Solo, None),
    ("in", Arity::Solo, None),
    ("out", Arity::Solo, None),
    ("examine", Arity::Object, None),
    ("take", Arity::Object, None),
    ("drop", Arity::Object, None),
    ("open", Arity::Object, None),
    ("close", Arity::Object, None),
    ("read", Arity::Object, None),
    ("eat", Arity::Object, None),
    ("drink", Arity::Object, None),
    ("wear", Arity::Object, None),
    ("remove", Arity::Object, None),
    ("turn", Arity::Object, None),
    ("enter", Arity::Object, None),
    ("lock", Arity::Pair, Some("with")),
    ("search", Arity::ObjectOpt, None),
    ("push", Arity::ObjectOpt, None),
    ("pull", Arity::ObjectOpt, None),
    ("climb", Arity::ObjectOpt, None),
    ("move", Arity::ObjectOpt, None),
    ("unlock", Arity::Pair, Some("with")),
    ("put", Arity::Pair, Some("in")),
    ("give", Arity::Pair, Some("to")),
    ("show", Arity::Pair, Some("to")),
    ("attack", Arity::Pair, Some("with")),
    ("tie", Arity::Pair, Some("to")),
];

/// The default one-click quick-action row.
///
/// The four diagonals sit with the rest of the compass (SQ-0676): the rose
/// draws all eight points, and leaving `ne`/`nw`/`se`/`sw` out left four of its
/// cells permanently empty. A game with no diagonal vocabulary (classic Scott
/// Adams) simply answers "I don't understand", exactly as it would to the same
/// word typed by hand — nothing here needs gating on the engine.
pub const DEFAULT_QUICK: &[&str] = &[
    "n", "s", "e", "w", "ne", "nw", "se", "sw", "up", "down", "in", "out", "look", "inventory",
    "wait", "again",
];

/// The built-in verb table as owned entries (the runtime form).
pub fn default_verbs() -> Vec<VerbEntry> {
    BUILTIN_VERBS.iter().map(|&(w, a, p)| VerbEntry::new(w, a, p)).collect()
}

/// The default quick row as owned strings.
pub fn default_quick() -> Vec<String> {
    DEFAULT_QUICK.iter().map(|s| s.to_string()).collect()
}

// ── Columns ──────────────────────────────────────────────────────────────────

/// Number of columns the band lays out: VERB, WHAT—here, WHAT—carried, and the
/// prepositional second-object column.
pub const BAND_COLS: usize = 4;

/// Column indices, named. The object slot is offered as TWO columns (here /
/// carried) because that split is the whole point of having live objects; both
/// fill the same grammatical slot.
pub const COL_VERB: usize = 0;
pub const COL_HERE: usize = 1;
pub const COL_CARRIED: usize = 2;
pub const COL_SECOND: usize = 3;

// ── Geometry ─────────────────────────────────────────────────────────────────

/// Rows the band occupies when fully open: the configured `height`, clamped
/// to [`MIN_BAND_ROWS`]..=[`MAX_BAND_ROWS`] and then to what the screen can
/// actually spare (never more than half of it, and never so much that the
/// story pane is left with nothing). 0 when the band isn't visible.
///
/// Post-SQ-0667 there is no frame and no phrase line, so every row here is
/// content — unlike the pre-amendment band, which spent 3 of its 8 default
/// rows on chrome (a 2-row border plus the phrase line) before a single verb
/// was visible.
pub fn band_target_height(visible: bool, full_height: u16, rows: u16) -> u16 {
    if !visible {
        return 0;
    }
    let want = rows.clamp(MIN_BAND_ROWS, MAX_BAND_ROWS);
    // Leave the help row plus at least 3 rows of story pane.
    let hi = full_height.saturating_sub(4);
    want.min(hi)
}

/// Smallest useful band: headers + one list row + quick row.
pub const MIN_BAND_ROWS: u16 = 3;
/// Largest band the config may ask for.
pub const MAX_BAND_ROWS: u16 = 11;
/// The shipped default band height, in rows. 5, not the pre-SQ-0667 8 — the
/// band lost its 2-row frame and 1-row phrase line, and this is picked to
/// show exactly as many list rows as the old default did (3).
pub const DEFAULT_BAND_ROWS: u16 = 5;

/// The reserved band height in rows: `target_h` scaled by the slide's current
/// `fraction` (0.0 closed .. 1.0 fully open). Mirrors `inventory_dock_height`.
pub fn band_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

/// Width of one vertical divider (SQ-0677): a single `│` cell, drawn between
/// the quick block and VERB and between every pair of adjacent columns.
pub const DIVIDER_W: u16 = 1;

/// Minimum width `column_rects` needs to lay out all `BAND_COLS` columns at
/// their own 6-cell floor PLUS a divider between every adjacent pair
/// (`BAND_COLS - 1` of them) — the same number `draw_command_band`'s
/// block-vs-fallback width check adds on top of the quick block's own width.
pub const MIN_COLS_WIDTH: u16 = BAND_COLS as u16 * 6 + (BAND_COLS as u16 - 1) * DIVIDER_W;

/// Split `content` (the band's inner rect, already narrowed past the quick
/// block and its divider if one is showing) into `BAND_COLS` column rects,
/// each separated by a `DIVIDER_W` gap the caller draws a `│` into
/// (`draw_command_band`). Returns an empty vec when the band is too narrow to
/// give each column a usable width — the divider cells are never part of any
/// returned rect, so a click that lands exactly on one hits neither neighbour.
pub fn column_rects(content: Rect) -> Vec<Rect> {
    if content.width < MIN_COLS_WIDTH {
        return Vec::new();
    }
    let dividers = BAND_COLS as u16 - 1;
    let usable = content.width - dividers * DIVIDER_W;
    let each = usable / BAND_COLS as u16;
    let mut rects = Vec::with_capacity(BAND_COLS);
    let mut x = content.x;
    for i in 0..BAND_COLS as u16 {
        rects.push(Rect { x, y: content.y, width: each, height: content.height });
        x += each;
        if i + 1 < BAND_COLS as u16 {
            x += DIVIDER_W;
        }
    }
    rects
}

/// Draw a full-band-height 1-cell vertical divider at column `x`. Reuses
/// `panel.border`'s style — the band draws no frame of its own (SQ-0667), but
/// a plain divider glyph is exactly what that selector already carries
/// elsewhere, and it is the same family `draw_command_band`'s resize-mode
/// highlight already borrows a color from (`panel.border:active`) — so no new
/// selector was needed for this (SQ-0677).
fn draw_divider(buf: &mut Buffer, x: u16, area: Rect, style: Style) {
    if x < area.x || x >= area.right() {
        return;
    }
    for y in area.y..area.bottom() {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol("\u{2502}").set_style(style);
        }
    }
}

// ── Live objects ─────────────────────────────────────────────────────────────

/// Refill the band's object columns from the engine.
///
/// The old verb menu snapshotted a transcript scrape at open and never looked
/// again; the band is LIVE. Called once per loop tick (and therefore at least
/// once per turn), so taking an object moves it from *here* to *carried* on the
/// very next frame. Cheap, and skipped entirely while the band is closed.
///
/// Z-machine gets the real object tree through `Introspect`. Glulx and Scott
/// have none, so `carried` falls back to the parsed-inventory snapshot and
/// `here` to the transcript scrape, flagged `here_is_seen` so the column labels
/// itself "seen" rather than passing a scrape off as the room's contents.
///
/// `here` excludes the player object itself (SQ-0667) — it is structurally a
/// child of whatever room the player is in, so without this it would show up
/// in every room of every game (Zork 1 lists "cretin", the adventurer's own
/// printed name). Excluded by id (`room_objects_excluding`), not by matching
/// the name, which could coincidentally collide with a real scenery object.
/// `examine me` still works by typing it; there is no dedicated row for it.
///
/// Returns `true` when the lists actually changed (→ repaint).
pub fn refresh_objects(state: &mut AppState, session: &dyn crate::engine::Engine) -> bool {
    if state.overlays.command_band.is_none() {
        return false;
    }
    let player = state
        .player_obj
        .or_else(|| session.introspect().and_then(|i| i.player_object()));
    let loc = session.current_location().map(|s| s.number).unwrap_or(0);

    let (here, carried, seen) = match session.introspect() {
        Some(intro) => {
            let here = if loc != 0 { intro.room_objects_excluding(loc, player) } else { Vec::new() };
            let carried = match player {
                Some(p) => intro.contents(p),
                None => state.inventory_fallback.clone(),
            };
            (here, carried, false)
        }
        None => (crate::input::scraped_seen_nouns(state), state.inventory_fallback.clone(), true),
    };

    let Some(band) = state.overlays.command_band.as_mut() else { return false };
    if band.here == here && band.carried == carried && band.here_is_seen == seen {
        return false;
    }
    band.here = here;
    band.carried = carried;
    band.here_is_seen = seen;
    true
}

// ── Hit rects ────────────────────────────────────────────────────────────────

/// Click targets emitted while drawing, for the event loop to hit-test.
#[derive(Default, Clone)]
pub struct CommandBandHits {
    /// The band's whole rect — clicks inside it belong to the band and must not
    /// reach the story pane / the v6 game.
    pub area: Rect,
    /// Column header rects, by column index (focuses that column).
    pub headers: Vec<(usize, Rect)>,
    /// Item rows, as `(column, index-within-the-filtered-list, rect)`.
    pub rows: Vec<(usize, usize, Rect)>,
    /// Quick-action words, as `(index into the quick list, rect)`.
    pub quick: Vec<(usize, Rect)>,
    /// Whole-column rects, for wheel routing.
    pub columns: Vec<(usize, Rect)>,
}

// ── Drawing ──────────────────────────────────────────────────────────────────

/// Draw the command band into `area` (the bottom band carved out by
/// `layout::compute_pane_layout` from the slide fraction).
///
/// Sets `*vp_out` to the ACTIVE column's visible list height so PageUp/PageDown
/// page by the right amount. No-op when the band is closed or `area` is too
/// small to show anything meaningful (mid-slide).
pub fn draw_command_band(
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    vp_out: &mut usize,
    hits: &mut CommandBandHits,
) {
    *hits = CommandBandHits::default();
    let Some(band) = &state.overlays.command_band else { return };
    hits.area = area;

    if area.width < 8 || area.height == 0 {
        return;
    }

    let theme = &state.colors.theme;
    // Borderless strip (SQ-0667, 2026-08-05): the band's 2-row frame and its
    // "Command" title are gone — the fill below is now the band's ONLY
    // visual separation from the story pane above it. Resize mode still
    // needs some affordance that the band is the live target; with no border
    // left to accent, the whole fill picks up `panel.border:active` instead.
    // …and equally while its top edge is being DRAGGED (SQ-0669). Hover is
    // deliberately excluded here, unlike the bordered panes: with no border to
    // accent, the whole band would flash every time the pointer crossed the row
    // above it on its way in.
    let resize_hl = (state.resize_mode
        && state.resize_target == crate::state::ResizeTarget::CommandBand)
        || matches!(state.pane_drag, Some(d) if d.boundary == crate::layout::Boundary::CommandBandTop);
    let base = if resize_hl {
        theme.get("panel.border:active").style
    } else {
        theme.get("dialog.background").style
    };

    // Opaque fill first, so panes behind never show through mid-slide.
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    let content = area;
    let divider_style = base.patch(theme.get("panel.border").style);
    let divider_active = base.patch(theme.get("panel.border:active").style);

    // ── Quick actions: a stacked rose+words block on the left edge when
    // there is room (SQ-0675, restacked under SQ-0677), else the flat
    // one-click row along the bottom (the original SQ-0667 layout, kept as
    // the narrow fallback). `use_block` narrows the columns by the block's
    // width plus one divider cell; see `draw_quick_block`'s doc for the
    // height interplay (the block's HEIGHT never steals from the columns,
    // which always get the band's full `content.height`). ─────────────────
    let quick_layout = if band.quick.is_empty() { None } else { Some(quick_block_layout(&band.quick)) };
    let use_block = quick_layout
        .as_ref()
        .is_some_and(|l| content.width >= l.width + DIVIDER_W + MIN_COLS_WIDTH);

    let mut cols_area = content;
    if use_block {
        let layout = quick_layout.as_ref().expect("use_block implies quick_layout is Some");
        let block_area = Rect {
            x: content.x,
            y: content.y,
            width: layout.width,
            height: content.height.min(layout.height),
        };
        draw_quick_block(band, layout, block_area, buf, base, theme, &mut hits.quick);
        let divider_x = content.x + layout.width;
        // The block/VERB divider is VERB's left flank: it carries the
        // current-column accent when VERB is current (see the column loop).
        let style = if band.focus == 0 { divider_active } else { divider_style };
        draw_divider(buf, divider_x, content, style);
        let claimed = layout.width + DIVIDER_W;
        cols_area = Rect {
            x: content.x + claimed,
            y: content.y,
            width: content.width - claimed,
            height: content.height,
        };
    } else if content.height >= 2 && !band.quick.is_empty() {
        let quick_area =
            Rect { x: content.x, y: content.bottom() - 1, width: content.width, height: 1 };
        draw_quick_row(band, quick_area, buf, base, theme, &mut hits.quick);
        cols_area =
            Rect { x: content.x, y: content.y, width: content.width, height: content.height - 1 };
    }

    // ── Columns ──────────────────────────────────────────────────────────────
    if cols_area.height == 0 || cols_area.width == 0 {
        *vp_out = 0;
        return;
    }
    // The band reads the real prompt: the word under construction there
    // highlights the nearest match in the CURRENT column (SQ-0677 scopes the
    // search to `band.focus`, which `Tab`/`Shift-Tab` move — see
    // `CommandBandState::nearest_match`'s doc for how that differs from the
    // SQ-0676 multi-column hunt it replaced).
    let typed = state.input.value.as_str();

    let rects = column_rects(cols_area);
    if rects.is_empty() {
        // Too narrow for four columns: show only the current one, full width.
        let highlight = band.highlighted_row(typed);
        draw_column(state, band, band.focus, cols_area, buf, base, true, highlight, vp_out, hits);
        return;
    }

    *vp_out = 0;
    for (col, rect) in rects.iter().enumerate() {
        hits.columns.push((col, *rect));
        let is_current = col == band.focus;
        let highlight = if is_current { band.highlighted_row(typed) } else { None };
        draw_column(state, band, col, *rect, buf, base, is_current, highlight, vp_out, hits);
    }
    // Dividers between adjacent columns — drawn last so they sit on top of
    // any column fill, full band height (not just the columns' own rect, in
    // case a shorter mid-slide `cols_area` ever diverges from `content`).
    // The CURRENT column's flanking dividers take the active accent — the
    // uniform current-column hint for all four columns (chrome, never list
    // rows; supersedes VERB's top-row underline, retired on user feedback).
    for (i, pair) in rects.windows(2).enumerate() {
        let flanks_current = band.focus == i || band.focus == i + 1;
        let style = if flanks_current { divider_active } else { divider_style };
        draw_divider(buf, pair[0].right(), content, style);
    }
}

// ── Compass-rose quick block (SQ-0675, restacked SQ-0677) ──────────────────
//
// The flat quick row (below) is the narrow-band fallback; when there is
// room, the quick actions instead draw as a block on the band's left edge:
// the compass rose on top, the non-compass words flowing below it —
// stacked, not side by side (SQ-0677; the SQ-0675 original packed both into
// the rose's own 3 rows, which left the block no narrower even when the
// word list was short, and ate width the columns could have used):
//
// ```text
//  NW  N  NE
//   W  ·  E
//  SW  S  SE
//  up   down
//  in    out
//  look   inv
//  wait again
// ```
//
// The 8 compass points (matched by MEANING via `parse_direction`, not
// spelling — same rule the VERB column's quick exclusion uses) form a 3×3
// rose with an always-inert centre; everything else in the effective quick
// list (up/down/in/out are directions too, but not compass POINTS, so they
// stay in the word list, not the rose) flows left-to-right then wraps,
// packed into as many rows as `WORD_ROW_BUDGET` needs at the narrowest width
// that achieves it. Every cell — rose point or word — is a `hits.quick`
// entry keyed by its index into `band.quick`, exactly like the flat row's
// words: this is a different LAYOUT of the same one-click-submit contract
// (`input::band_quick_pick_command`, `main::band_mouse_action`), not a new
// one, so neither needed to change.
//
// **Height interplay (SQ-0677):** the block's natural height (rose rows plus
// however many word rows `WORD_ROW_BUDGET` needs) may exceed the band's
// actual configured height — e.g. the shipped default (5 rows) fits the
// 3-row rose plus only 2 of the default quick list's 3 word rows. Of the
// three options on the table (cap the block at the band's height and clip
// the rest; wrap overflow words into an extra COLUMN of the block; raise the
// band's effective minimum height whenever the block is shown), the first is
// what's implemented: `draw_command_band` sizes `block_area.height` to
// `content.height.min(layout.height)`, and `draw_quick_block` below silently
// stops drawing (and registering hits for) any row that falls off the
// bottom — the exact same clip-past-the-edge the renderer already did at a
// fixed 3 rows before this amendment, just against a height that now varies
// with the word list instead of a constant. This was picked over the other
// two because it costs nothing elsewhere (the columns still always get the
// band's full height; `MIN_BAND_ROWS`/`DEFAULT_BAND_ROWS` stay singular,
// global constants unaffected by which quick list happens to be configured)
// and a taller band is one resize away for anyone who wants every word row
// visible at once.

/// Target row budget the word flow tries to pack into (`word_flow_width`
/// grows the width, not this) — independent of the band's actual configured
/// height; see the height-interplay note above for why. Picked so the whole
/// block (rose + words) lands around the "3 rose + a handful of word rows"
/// shape a taller-than-default band comfortably shows in full.
const WORD_ROW_BUDGET: u16 = 3;
/// Rows the compass rose itself occupies.
const ROSE_ROWS: u16 = 3;
/// Width of one rose cell: enough for the two-letter intercardinal labels
/// (`NW`/`NE`/`SW`/`SE`); the one-letter `N`/`S`/`E`/`W` and the centre `·`
/// are right-justified into the same width.
const ROSE_CELL_W: u16 = 2;
/// Columns between adjacent rose cells.
const ROSE_GAP: u16 = 1;
/// Total width of the 3×3 rose grid: three cells plus the two gaps between them.
const ROSE_WIDTH: u16 = ROSE_CELL_W * 3 + ROSE_GAP * 2;
/// Left margin before the block, and the gap separating it from the columns
/// that follow — matches the flat quick row's own one-column left margin.
const BLOCK_MARGIN: u16 = 1;

/// The rose's 8 outer grid slots, reading order (row-major, matching the
/// sketch above): NW N NE / W · E / SW S SE, with the centre omitted (it is
/// never a pick target).
const ROSE_LABELS: [&str; 8] = ["NW", "N", "NE", "W", "E", "SW", "S", "SE"];
const ROSE_ORDER: [mapper::direction::Direction; 8] = {
    use mapper::direction::Direction::*;
    [NW, N, NE, W, E, SW, S, SE]
};

/// A resolved rose+words quick block: which `band.quick` index (if any)
/// fills each of the 8 rose slots, whether the rose draws at all, the word
/// flow's row assignment, and the total width/height the whole block needs —
/// computed once so the width-vs-fallback decision in `draw_command_band` and
/// the actual drawing in `draw_quick_block` always agree exactly.
struct QuickBlockLayout {
    /// `band.quick` index for each of the 8 rose slots (`ROSE_LABELS` order).
    rose: [Option<usize>; 8],
    has_rose: bool,
    /// One row per line of the word flow, each a list of `(index into
    /// band.quick, x-offset within the block, past `BLOCK_MARGIN`)`. Empty
    /// when the effective quick list has no non-compass words at all.
    word_rows: Vec<Vec<(usize, u16)>>,
    /// Row (from the block's top) where the word flow starts: `ROSE_ROWS`
    /// when the rose draws (words stack UNDER it), else 0 (no rose to stack
    /// under).
    words_y: u16,
    /// Total width the block needs, margins included: `BLOCK_MARGIN` +
    /// `max(rose width, widest word row)` + `BLOCK_MARGIN` — "as narrow as
    /// the widest word row" whenever that's wider than the rose.
    width: u16,
    /// Total height the block wants: `words_y` + the word flow's row count.
    /// May exceed the band's actual content height; see the height-interplay
    /// note above `WORD_ROW_BUDGET` for how the caller handles that.
    height: u16,
}

/// Split the effective quick list into the 8 compass-rose slots (by index, so
/// a click can resolve through the same `band.quick.get(idx)` every other
/// pick does) and everything else, in original list order. A word is routed
/// to the rose by the DIRECTION it names (`parse_direction`), not its
/// spelling, so a custom quick row spelling out `"north"` still lands in the
/// rose's N slot rather than the word flow — the same rule
/// `CommandBandState::items`'s VERB exclusion already uses. `up`/`down`/`in`/
/// `out` are directions too but not compass POINTS, so `ROSE_ORDER` excludes
/// them on purpose — they flow as ordinary words instead, per the design.
fn split_quick_rose(quick: &[String]) -> ([Option<usize>; 8], Vec<usize>) {
    use mapper::direction::parse_direction;
    let mut rose: [Option<usize>; 8] = [None; 8];
    let mut words = Vec::new();
    for (i, w) in quick.iter().enumerate() {
        let slot = parse_direction(w).and_then(|d| ROSE_ORDER.iter().position(|&r| r == d));
        match slot {
            Some(s) => rose[s] = Some(i),
            None => words.push(i),
        }
    }
    (rose, words)
}

/// Rows a greedy, row-major left-to-right wrap of `words` needs at `width`: a
/// word starts a new row only when appending it (with a one-column separator)
/// would overflow the current row.
fn rows_needed(words: &[&str], width: u16) -> u16 {
    let mut rows = 1u16;
    let mut cur = 0u16;
    for word in words {
        let wl = word.chars().count() as u16;
        let extended = if cur == 0 { wl } else { cur + 1 + wl };
        if extended > width && cur > 0 {
            rows += 1;
            cur = wl;
        } else {
            cur = extended;
        }
    }
    rows
}

/// The narrowest width that packs `words` into `budget` rows under
/// `rows_needed`'s wrap — grown one column at a time from the longest single
/// word (the floor: nothing narrower could ever place that word). Small `n`
/// (the quick list is a handful of words at most) makes the linear search
/// cheap enough to redo on every draw rather than cache. By construction the
/// width this settles on IS the widest packed row's rendered width — which is
/// what makes it "as narrow as the widest word row" (the block-width doc
/// above `WORD_ROW_BUDGET` promises).
fn word_flow_width(words: &[&str], budget: u16) -> u16 {
    let Some(mut w) = words.iter().map(|s| s.chars().count() as u16).max() else { return 0 };
    while rows_needed(words, w) > budget {
        w += 1;
    }
    w
}

/// Assign each word (given as `band.quick` indices) to a row and an x-offset
/// within the word-flow area, using the exact wrap `word_flow_width` sized
/// the area for. Empty `idxs` produces zero rows (not one empty row) — the
/// height math above (`words_y + word_rows.len()`) depends on that: a
/// compass-only custom quick list must not pay for a phantom blank row.
fn flow_words(quick: &[String], idxs: &[usize], width: u16) -> Vec<Vec<(usize, u16)>> {
    let mut rows: Vec<Vec<(usize, u16)>> = Vec::new();
    let mut cur = 0u16;
    for &i in idxs {
        let wl = quick[i].chars().count() as u16;
        let extended = if cur == 0 { wl } else { cur + 1 + wl };
        if rows.is_empty() || (extended > width && cur > 0) {
            rows.push(Vec::new());
            cur = 0;
        }
        let x = if cur == 0 { 0 } else { cur + 1 };
        rows.last_mut().expect("just pushed if empty").push((i, x));
        cur = x + wl;
    }
    rows
}

/// Resolve `quick` into a [`QuickBlockLayout`]. Pure and cheap (see
/// `word_flow_width`'s doc) — called from `draw_command_band` purely to
/// answer the width-vs-fallback question, then again (necessarily the same
/// answer) to actually draw.
fn quick_block_layout(quick: &[String]) -> QuickBlockLayout {
    let (rose, word_idxs) = split_quick_rose(quick);
    let has_rose = rose.iter().any(Option::is_some);
    let word_strs: Vec<&str> = word_idxs.iter().map(|&i| quick[i].as_str()).collect();
    let words_width = word_flow_width(&word_strs, WORD_ROW_BUDGET);
    let word_rows = flow_words(quick, &word_idxs, words_width);

    // Rose and words now share the same left edge and stack vertically
    // (SQ-0677), so the block's width is just whichever of the two needs
    // more room — no horizontal gap between them left to account for.
    let content_w = if has_rose { ROSE_WIDTH.max(words_width) } else { words_width };
    let width = BLOCK_MARGIN + content_w + BLOCK_MARGIN;
    let words_y = if has_rose { ROSE_ROWS } else { 0 };
    let height = words_y + word_rows.len() as u16;
    QuickBlockLayout { rose, has_rose, word_rows, words_y, width, height }
}

/// Draw the rose+words quick block into `area` (the band's left strip, sized
/// by the caller to `layout.width` × `content.height.min(layout.height)` —
/// see the height-interplay note above `WORD_ROW_BUDGET`).
///
/// Any row — rose or word — that falls at or past `area.bottom()` is simply
/// not drawn and registers no `hits.quick` entry: a short band shows the rose
/// and clips the word rows it has no room for, exactly the clip-past-the-edge
/// behaviour the pre-SQ-0677 renderer already had at a fixed 3 rows.
///
/// Every rose point and every word registers the exact same `hits` entry
/// shape the flat row's words did (`(index into band.quick, rect)`), so
/// `input::band_quick_pick_command` and `main::band_mouse_action` need no
/// changes — a click resolves identically regardless of which layout drew it.
fn draw_quick_block(
    band: &crate::state::CommandBandState,
    layout: &QuickBlockLayout,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    theme: &crate::theme::resolve::Theme,
    hits: &mut Vec<(usize, Rect)>,
) {
    let style = base.patch(theme.get("band.quick").style);
    // The centre is always inert decoration — never a pick target — styled
    // like the map matrix's own frontier dot rather than a new selector.
    let dim = base.patch(theme.get("map.matrix.cell:frontier").style);
    // Hover is the quick block's ONLY transient highlight now (SQ-0677 made
    // it mouse-click-only, retiring the arrow-armed selection this cell style
    // used to carry) — reversed video, deliberately distinct from the column
    // rows' `dialog.list_selected` (a fg/bg swap, not REVERSED — see the
    // design doc amendment for why reversed was safe to pick).
    let cell_style = |qi: usize| {
        if band.quick_hover == Some(qi) {
            style.patch(theme.get("band.quick:hover").style)
        } else {
            style
        }
    };

    if layout.has_rose {
        for (cell, label) in ROSE_LABELS.iter().enumerate() {
            // Reading order is row-major over the 3×3 grid with the centre
            // (grid index 4) skipped, so cells 0..4 map to grid cells 0..4
            // and cells 4..8 map to grid cells 5..9.
            let grid = if cell < 4 { cell } else { cell + 1 };
            let (row, gcol) = ((grid / 3) as u16, (grid % 3) as u16);
            let y = area.y + row;
            if y >= area.bottom() {
                continue;
            }
            let x = area.x + BLOCK_MARGIN + gcol * (ROSE_CELL_W + ROSE_GAP);
            let cell_area = Rect { x, y, width: ROSE_CELL_W, height: 1 };
            let Some(qi) = layout.rose[cell] else { continue };
            let lx = x + ROSE_CELL_W.saturating_sub(label.chars().count() as u16);
            let st = cell_style(qi);
            if band.quick_hover == Some(qi) {
                for cx in cell_area.x..cell_area.right() {
                    if let Some(c) = buf.cell_mut((cx, y)) {
                        c.set_symbol(" ").set_style(st);
                    }
                }
            }
            crate::render::draw_str_clipped(buf, lx, y, label, st, cell_area);
            hits.push((qi, cell_area));
        }
        // The centre dot, row 1 / col 1 of the grid.
        let cy = area.y + 1;
        if cy < area.bottom() {
            let cx = area.x + BLOCK_MARGIN + (ROSE_CELL_W + ROSE_GAP) + ROSE_CELL_W - 1;
            if let Some(c) = buf.cell_mut((cx, cy)) {
                c.set_symbol("\u{b7}").set_style(dim);
            }
        }
    }

    for (row_i, row) in layout.word_rows.iter().enumerate() {
        let y = area.y + layout.words_y + row_i as u16;
        if y >= area.bottom() {
            break;
        }
        for &(qi, x_off) in row {
            let word = &band.quick[qi];
            let x = area.x + BLOCK_MARGIN + x_off;
            if x >= area.right() {
                continue;
            }
            let w = (word.chars().count() as u16).min(area.right() - x);
            let r = Rect { x, y, width: w, height: 1 };
            crate::render::draw_str_clipped(buf, x, y, word, cell_style(qi), area);
            hits.push((qi, r));
        }
    }
}

/// The one-click quick-action row. SQ-0667 amendment (2026-08-05): unlike
/// every other pick, these fire AT ONCE — no Enter, and they don't compose
/// onto the input line either (see `input::band_quick_pick_command`).
fn draw_quick_row(
    band: &crate::state::CommandBandState,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    theme: &crate::theme::resolve::Theme,
    hits: &mut Vec<(usize, Rect)>,
) {
    let style = base.patch(theme.get("band.quick").style);
    let hover_style = theme.get("band.quick:hover").style;
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
    let mut x = area.x + 1;
    for (i, word) in band.quick.iter().enumerate() {
        let w = word.chars().count() as u16;
        if x + w > area.right() {
            break;
        }
        let r = Rect { x, y: area.y, width: w, height: 1 };
        // Hover reads here too (SQ-0677) — same reversed style the rose+words
        // block uses; the flat row is only ever a fallback for the same block.
        let st = if band.quick_hover == Some(i) { style.patch(hover_style) } else { style };
        crate::render::draw_str_clipped(buf, x, area.y, word, st, area);
        hits.push((i, r));
        x += w + 1;
    }
}

/// One column: its header row plus the (scrolled) item list.
///
/// `is_current` is whether `col` is `band.focus` — the ONE column `Tab`/
/// `Shift-Tab` point at (SQ-0677) — which is what lights the header (or, for
/// VERB, underlines its top row; see below) and drives `vp_out`. `highlight`
/// is the row to mark with `▸` and the selected style: the caller only ever
/// passes one for the current column (`CommandBandState::highlighted_row`),
/// so a non-current column never shows a highlight even if it happens to
/// have a passing nearest-match text.
#[allow(clippy::too_many_arguments)]
fn draw_column(
    state: &AppState,
    band: &crate::state::CommandBandState,
    col: usize,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    is_current: bool,
    highlight: Option<usize>,
    vp_out: &mut usize,
    hits: &mut CommandBandHits,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = &state.colors.theme;
    let reachable = band.col_reachable(col);

    // SQ-0675: the VERB column's header row carried no text at all (SQ-0667
    // dropped the "VERB" label, leaving the row visually blank) — pure
    // wasted space in a deliberately compact band. It is reclaimed here as
    // the column's first LIST row instead of being drawn separately, so the
    // VERB column shows exactly ONE MORE visible verb than the object/prep
    // columns show items, at the same overall column height. Every other
    // column still gets a real header row: WHAT — here / WHAT — carried /
    // WITH… all carry real information (here vs. carried, and the pair
    // verb's preposition), so their row stays worth spending on a label.
    // VERB also contributes no `hits.headers` entry — there is no separate
    // header region left to focus; clicking its reclaimed top row just picks
    // the first verb there, exactly like clicking any other row.
    //
    // The current-column hint decorates CHROME, not list rows: the dividers
    // flanking the current column take `panel.border:active` (see the divider
    // loop in `draw_command_band`), uniformly for all four columns. VERB
    // therefore needs nothing special here despite having no header row —
    // its earlier top-row underline read as "this entry is underlined for
    // some reason" and was retired on user feedback (2026-08-05).
    let header_h: u16 = if col == COL_VERB { 0 } else { 1 };
    if header_h > 0 {
        let header_style = base.patch(
            theme.get(if is_current { "band.column_header:active" } else { "band.column_header" }).style,
        );
        let header_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
        for x in header_area.x..header_area.right() {
            if let Some(cell) = buf.cell_mut((x, header_area.y)) {
                cell.set_symbol(" ").set_style(header_style);
            }
        }
        let label = format!("{}{}", if is_current { "▸" } else { " " }, band.column_label(col));
        crate::render::draw_str_clipped(buf, header_area.x, header_area.y, &label, header_style, header_area);
        hits.headers.push((col, header_area));
    }
    let list_h = area.height.saturating_sub(header_h);
    if is_current {
        *vp_out = list_h as usize;
    }
    if list_h == 0 {
        return;
    }
    let list_area = Rect { x: area.x, y: area.y + header_h, width: area.width, height: list_h };

    // A column that is not reachable yet renders dimmed and empty — the grammar
    // is the point, so showing pickable-looking rows there would lie.
    if !reachable {
        return;
    }

    let items = band.items(col);
    let label_style = base.patch(theme.get("band.group_label").style);
    if items.is_empty() {
        // Column-specific wording (SQ-0667, following the SQ-0668 data fix
        // that made an empty carried/here column a real possibility rather
        // than blank space that reads as broken): "(nothing visible)" stays
        // the fallback for VERB/SECOND, which are never truly this empty in
        // practice (VERB always has the built-in table; SECOND only opens
        // once a pair verb's first object is picked, from the union of the
        // very lists this message would otherwise be reporting on for).
        let msg = match col {
            COL_HERE => "(nothing here)",
            COL_CARRIED => "(nothing carried)",
            _ => "(nothing visible)",
        };
        crate::render::draw_str_clipped(buf, list_area.x, list_area.y, msg, label_style, list_area);
        return;
    }

    let scroll = &band.scroll[col];
    let visible = list_area.height as usize;
    let total = items.len();
    // Only the nearest match highlights (SQ-0676). `usize::MAX` = nothing in
    // this column matches, so no row wears the marker at all — an unmatched
    // column must not look like it is offering a pick.
    let selected = highlight.unwrap_or(usize::MAX);
    let scrollbar_visible =
        crate::render::scroll::needs_scrollbar(total, visible) && list_area.width >= 2;
    let row_w = if scrollbar_visible { list_area.width.saturating_sub(1) } else { list_area.width };
    let offset = scroll.display_offset().min(total.saturating_sub(1));

    for row in 0..visible {
        let idx = offset + row;
        let y = list_area.y + row as u16;
        if y >= list_area.bottom() || idx >= total {
            break;
        }
        let is_selected = idx == selected;
        let style = if is_selected { theme.get("dialog.list_selected").style } else { base };
        let marker = if is_selected { "▸" } else { " " };
        let line = format!("{}{}", marker, items[idx]);
        let row_area = Rect::new(list_area.x, y, row_w, 1);
        hits.rows.push((col, idx, row_area));
        for x in row_area.x..row_area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        crate::render::draw_str_clipped(buf, row_area.x, y, &line, style, row_area);
    }

    if scrollbar_visible {
        let sb = Rect::new(list_area.right().saturating_sub(1), list_area.y, 1, list_area.height);
        crate::render::scroll::draw_scrollbar(
            buf,
            sb,
            total,
            visible,
            scroll.target_offset(),
            crate::render::scroll::ScrollbarLook::from_theme(theme),
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, CommandBandState};

    // Wide enough that the default quick list's compass-rose block (SQ-0675)
    // fits alongside four columns with real breathing room (so header/list
    // text like "(nothing carried)" never clips) — most of this module's
    // tests care about column content, not the quick block's own layout;
    // `narrow_band_falls_back_to_the_flat_quick_row` below covers the
    // narrower case on purpose.
    const BAND: Rect = Rect { x: 0, y: 0, width: 120, height: 8 };

    fn state_with_band() -> AppState {
        let mut s = AppState::default();
        let mut band = CommandBandState::new(default_verbs(), default_quick());
        band.here = vec!["iron door".to_string(), "mailbox".to_string()];
        band.carried = vec!["brass key".to_string(), "lantern".to_string()];
        s.overlays.command_band = Some(band);
        s
    }

    fn dump(buf: &Buffer) -> String {
        buf.content().iter().map(|c| c.symbol().to_owned()).collect()
    }

    #[test]
    fn band_shows_verbs_and_live_objects() {
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        // `take`, not `look` — `look` is in the default quick row, so SQ-0667
        // excludes it from the VERB column (see
        // `verb_column_excludes_quick_words` in `state.rs`).
        assert!(out.contains("take"), "a verb");
        assert!(out.contains("carried"), "the carried column header");
    }

    #[test]
    fn unreachable_columns_render_no_rows() {
        // With no verb picked, only the VERB column is reachable, so no object
        // rows may be emitted as click targets.
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(hits.rows.iter().all(|(c, _, _)| *c == COL_VERB), "only verb rows are pickable");
        assert!(!dump(&buf).contains("iron door"), "objects are not offered before a verb");
    }

    #[test]
    fn object_columns_open_after_an_object_verb() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(out.contains("iron door"), "here objects become pickable");
        assert!(out.contains("brass key"), "carried objects become pickable");
        assert!(hits.rows.iter().any(|(c, _, _)| *c == COL_HERE));
    }

    /// SQ-0667 (2026-08-05): the band no longer draws its own frame, title, or
    /// phrase line — composing now happens on the real story input line
    /// (`state.input`), drawn elsewhere. Falsifies against the pre-amendment
    /// band, which drew "Command" in a title strip and "Enter: send" on a
    /// dedicated phrase row.
    #[test]
    fn band_no_longer_draws_a_frame_or_phrase_line() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("look");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(!out.contains("Command"), "the title strip is retired along with the frame");
        assert!(!out.contains("Enter: send"), "the armed/send affordance moved to the real prompt");
        assert!(!out.contains('┌') && !out.contains('└'), "no border corners — a borderless strip");
    }

    /// SQ-0667: the VERB column shows no header text (it's self-evident); the
    /// object columns keep theirs, since WHAT/WITH carry real information.
    #[test]
    fn verb_column_header_carries_no_text() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(!dump(&buf).contains("VERB"), "the VERB label is gone");
        // SQ-0675: the header ROW itself is gone for VERB too now — reclaimed
        // as an extra list row (see `verb_column_shows_one_more_row...`
        // below) — so only the other three columns still contribute a
        // `hits.headers` entry.
        assert_eq!(
            hits.headers.len(),
            BAND_COLS - 1,
            "every column but VERB still has a clickable header row"
        );
    }

    /// SQ-0675: the VERB column's blank header row is reclaimed as its FIRST
    /// list row, so at the SAME column height it shows exactly one more
    /// visible item than a column that still spends a row on a real header
    /// (WHAT — here). Both columns are seeded with far more items than the
    /// area can show, so each is "full" and its row count is governed purely
    /// by how many rows `draw_column` gives its list. Falsifies against
    /// reverting the `header_h` special-case in `draw_column` back to an
    /// unconditional 1, which would make the two counts equal.
    #[test]
    fn verb_column_shows_one_more_row_than_an_object_column_at_the_same_height() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take"); // opens HERE/CARRIED
            b.here = (0..10).map(|i| format!("thing{i}")).collect();
        }
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        let verb_rows = hits.rows.iter().filter(|(c, _, _)| *c == COL_VERB).count();
        let here_rows = hits.rows.iter().filter(|(c, _, _)| *c == COL_HERE).count();
        assert!(verb_rows > 0 && here_rows > 0, "both lists are long enough to fill every row");
        assert_eq!(
            verb_rows,
            here_rows + 1,
            "VERB's reclaimed header row is one extra visible verb: verb={verb_rows} here={here_rows}"
        );
    }

    #[test]
    fn prep_column_header_names_the_preposition() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("unlock");
            b.pick(COL_HERE, 0);
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(dump(&buf).contains("WITH"), "the second-object column names its preposition");
    }

    /// The quick actions are hit-testable regardless of which layout drew
    /// them — the compass-rose block (SQ-0675, `BAND` is wide enough to fit
    /// it) or the flat row it falls back to (see
    /// `narrow_band_falls_back_to_the_flat_quick_row` for that path
    /// specifically).
    #[test]
    fn quick_row_is_hit_testable() {
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(!hits.quick.is_empty(), "quick words emit hit rects");
        assert_eq!(hits.headers.len(), BAND_COLS - 1, "every column but VERB has a clickable header row");
    }

    /// SQ-0675: too narrow for the rose+words block (plus a usable four
    /// columns), the quick actions fall back to the original flat one-click
    /// row along the bottom — the pre-SQ-0675 layout, still exercised here.
    /// Falsifies against a `use_block` that ignores width entirely (which
    /// would draw the rose even into a sliver this narrow, clipping it).
    #[test]
    fn narrow_band_falls_back_to_the_flat_quick_row() {
        let s = state_with_band(); // default quick: n s e w up down in out look inventory wait again
        let area = Rect { x: 0, y: 0, width: 40, height: 8 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        assert!(!hits.quick.is_empty(), "quick words still emit hit rects");
        let out = dump(&buf);
        assert!(!out.contains("NW") && !out.contains("SE"), "too narrow for the rose diagram: {out}");
        // Early quick words only — width=40 isn't enough for the whole flat
        // row (the flat row breaks off once it runs out of space, same as it
        // always has), so don't assert on the tail end of the list.
        assert!(out.contains("down"), "the flat row still spells out the quick words: {out}");
        assert!(
            hits.quick.iter().all(|(_, r)| r.y == area.bottom() - 1),
            "the flat row is a single strip along the very bottom: {:?}",
            hits.quick
        );
    }

    /// SQ-0675: with room, the quick actions draw as a compass rose (8 outer
    /// cells, matched by MEANING via `parse_direction`) plus a word block —
    /// every cell is its own click target, resolving through the same
    /// `band_quick_pick_command` lookup the flat row always used (directions
    /// spell out in full on submission; the rose only abbreviates the
    /// DISPLAY). Falsifies against `draw_quick_block` not registering a hit
    /// for a rose cell (would leave `hits.quick.len()` short of `quick.len()`).
    #[test]
    fn rose_cells_are_present_and_clickable_when_width_allows() {
        let quick: Vec<String> =
            ["nw", "n", "ne", "w", "e", "sw", "s", "se", "look"].iter().map(|s| s.to_string()).collect();
        let mut s = AppState::default();
        s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick.clone()));
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);

        assert_eq!(hits.quick.len(), quick.len(), "every rose cell and word is clickable: {:?}", hits.quick);

        // No two quick hit rects overlap.
        for (i, (_, a)) in hits.quick.iter().enumerate() {
            for (_, b) in hits.quick.iter().skip(i + 1) {
                let overlap = a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom();
                assert!(!overlap, "quick hit rects must not overlap: {a:?} vs {b:?}");
            }
        }

        // Each hit resolves through the real submission lookup to the right
        // command — directions spell out in full, "look" passes through.
        let mut resolved: Vec<String> = hits
            .quick
            .iter()
            .map(|(idx, _)| crate::input::band_quick_pick_command(&s, *idx).expect("valid index"))
            .collect();
        resolved.sort();
        let mut want = vec![
            "north", "south", "east", "west", "northeast", "northwest", "southeast", "southwest", "look",
        ];
        want.sort();
        assert_eq!(resolved, want);
    }

    /// The word flow holds exactly the effective quick words that are NOT one
    /// of the 8 compass points — `up`/`down`/`in`/`out` are directions too,
    /// but not compass POINTS, so they stay in the word flow rather than the
    /// rose (see `split_quick_rose`'s doc). Falsifies against a rose that
    /// (wrongly) also swallows up/down/in/out, which would shrink this list.
    #[test]
    fn word_block_holds_exactly_the_non_compass_quick_words() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.has_rose, "n/s/e/w are compass words — the rose shows");

        let mut got: Vec<&str> =
            layout.word_rows.iter().flatten().map(|&(i, _)| quick[i].as_str()).collect();
        got.sort_unstable();
        let mut want = vec!["up", "down", "in", "out", "look", "inventory", "wait", "again"];
        want.sort_unstable();
        assert_eq!(got, want, "the word flow holds exactly the non-compass quick words");
    }

    /// A custom quick list with no compass words at all draws no rose —
    /// just the flowing word list, starting where the rose's margin would
    /// otherwise have been. Falsifies against a rose that always draws (even
    /// empty), which would waste the rose's width for nothing.
    #[test]
    fn quick_list_without_compass_words_shows_no_rose() {
        let quick: Vec<String> = ["look", "wait", "again"].iter().map(|s| s.to_string()).collect();
        let layout = quick_block_layout(&quick);
        assert!(!layout.has_rose, "no compass word in quick -> no rose");
        assert_eq!(layout.words_y, 0, "no rose to stack the word flow under, so it starts at row 0");

        let s = {
            let mut s = AppState::default();
            s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick));
            s
        };
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(!out.contains('·'), "no rose -> no inert centre dot either: {out}");
        assert_eq!(hits.quick.len(), 3, "just the three words: {:?}", hits.quick);
    }

    /// SQ-0676 added the four diagonals to the built-in quick list, so the
    /// default rose has no empty cells left. Falsifies against the pre-SQ-0676
    /// `DEFAULT_QUICK`, where NW/NE/SW/SE were blank.
    #[test]
    fn the_default_quick_list_fills_all_eight_rose_cells() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.rose.iter().all(Option::is_some), "every compass point has a word");

        let mut s = AppState::default();
        s.overlays.command_band = Some(CommandBandState::new(default_verbs(), quick.clone()));
        let area = Rect { x: 0, y: 0, width: 120, height: 6 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        for label in ["NW", "NE", "SW", "SE"] {
            assert!(out.contains(label), "the rose draws {label}: {out}");
        }
        // …and a diagonal submits spelled out, like every other direction.
        let ne = quick.iter().position(|q| q == "ne").expect("ne is in the default quick list");
        assert_eq!(crate::input::band_quick_pick_command(&s, ne), Some("northeast".to_string()));
    }

    /// SQ-0676: the word being typed at the PROMPT highlights the nearest match
    /// — the band reads the input line rather than owning a filter of its own.
    /// Falsifies against a band that highlights `scroll.selected` regardless of
    /// what is typed (the pre-SQ-0676 behaviour), which marked row 0 of the
    /// focused column no matter what.
    #[test]
    fn the_typed_word_highlights_the_nearest_match() {
        let mut s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(!dump(&buf).contains('▸'), "nothing typed -> nothing highlighted");

        // `examine` heads the VERB column, so it is on screen without relying
        // on the scroll-to-match `apply_action` does for the live app.
        s.input.set("exa".to_string(), true);
        s.overlays.command_band.as_mut().unwrap().sync_from_input(&s.input.value);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(dump(&buf).contains("▸examine"), "the nearest verb wears the marker");

        s.input.set("zzzz".to_string(), true);
        s.overlays.command_band.as_mut().unwrap().sync_from_input(&s.input.value);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(!dump(&buf).contains('▸'), "no match -> no highlight at all");
    }

    /// The hovered quick word is visibly restyled (SQ-0677: quick lost its
    /// arrow-armed keyboard state and gained mouse hover as its one
    /// transient highlight instead). Falsifies against drawing every quick
    /// cell in the same `band.quick` style regardless of `quick_hover`.
    #[test]
    fn the_hovered_quick_word_is_drawn_differently() {
        let s_plain = state_with_band();
        let mut buf_plain = Buffer::empty(BAND);
        draw_command_band(&s_plain, BAND, &mut buf_plain, &mut 0, &mut CommandBandHits::default());

        let mut s_hover = state_with_band();
        s_hover.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf_hover = Buffer::empty(BAND);
        draw_command_band(&s_hover, BAND, &mut buf_hover, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_plain.content(),
            buf_hover.content(),
            "hovering a quick word must show somewhere on screen"
        );
    }

    /// The hover style is REVERSED video — deliberately distinct from the
    /// column rows' `dialog.list_selected` (a fg/bg swap, not a REVERSED
    /// modifier — see `theme::registry`), so hovering a quick cell can never
    /// be mistaken for a column row's picked/armed highlight even though
    /// both can be visible at once (hover in the quick block, the highlight
    /// in a column). Falsifies against a hover style that happens to collide
    /// with `dialog.list_selected`'s own modifiers.
    #[test]
    fn hover_style_is_reversed_and_distinct_from_the_column_selected_style() {
        use ratatui::style::Modifier;
        let mut s = state_with_band();
        s.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);

        let (_, r) = hits.quick.iter().find(|(i, _)| *i == 0).expect("hovered cell has a hit rect");
        let cell = buf.cell((r.x, r.y)).expect("hovered cell drawn");
        assert!(
            cell.style().add_modifier.contains(Modifier::REVERSED),
            "the hovered cell is reversed video: {:?}",
            cell.style()
        );

        let sel_style = s.colors.theme.get("dialog.list_selected").style;
        assert!(
            !sel_style.add_modifier.contains(Modifier::REVERSED),
            "sanity: the column-row selected style is a colour swap, not REVERSED — \
             otherwise hover and armed would be indistinguishable"
        );
    }

    /// The flat-row fallback hovers too (SQ-0677) — the block is the mouse's
    /// only path to the quick actions when it's showing, so hover must not
    /// be a rose-only affordance.
    #[test]
    fn the_flat_row_hovers_too() {
        let s = state_with_band();
        let area = Rect { x: 0, y: 0, width: 40, height: 8 };
        let mut buf_plain = Buffer::empty(area);
        draw_command_band(&s, area, &mut buf_plain, &mut 0, &mut CommandBandHits::default());

        let mut s_hover = state_with_band();
        s_hover.overlays.command_band.as_mut().unwrap().quick_hover = Some(0);
        let mut buf_hover = Buffer::empty(area);
        draw_command_band(&s_hover, area, &mut buf_hover, &mut 0, &mut CommandBandHits::default());

        assert_ne!(buf_plain.content(), buf_hover.content(), "the flat row's hover shows too");
    }

    #[test]
    fn seen_fallback_relabels_the_here_column() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here_is_seen = true;
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(out.contains("seen"), "the scraped fallback labels itself 'seen'");
    }

    /// A genuinely empty carried/here column must say so explicitly, not
    /// render as blank space that reads as broken (SQ-0668 made an empty
    /// column, with real data now behind it, an actual possibility — not
    /// just a scrape-fallback artifact).
    #[test]
    fn empty_object_columns_say_so() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.here = Vec::new();
            b.carried = Vec::new();
            b.pick_word("take");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(out.contains("nothing here"), "the here column says it's empty: {out}");
        assert!(out.contains("nothing carried"), "the carried column says it's empty: {out}");
    }

    /// …and the message is gone the moment either column actually has
    /// something in it — falsifies against always showing the empty row.
    #[test]
    fn nonempty_object_columns_do_not_say_empty() {
        let mut s = state_with_band(); // here/carried both non-empty by construction
        s.overlays.command_band.as_mut().unwrap().pick_word("take");
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(!out.contains("nothing here"), "the here column has objects: {out}");
        assert!(!out.contains("nothing carried"), "the carried column has objects: {out}");
    }

    #[test]
    fn band_is_opaque() {
        let mut buf = Buffer::empty(BAND);
        for y in 0..BAND.height {
            for x in 0..BAND.width {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol("X").set_style(
                        Style::new()
                            .fg(ratatui::style::Color::Red)
                            .bg(ratatui::style::Color::Green),
                    );
                }
            }
        }
        let s = state_with_band();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(
            !buf.content().iter().any(|c| c.style().bg == Some(ratatui::style::Color::Green)),
            "the band paints over whatever was behind it"
        );
    }

    #[test]
    fn tiny_band_does_not_panic() {
        for (w, h) in [(4u16, 8u16), (78, 2), (10, 3), (0, 0)] {
            let area = Rect { x: 0, y: 0, width: w, height: h };
            let mut buf = Buffer::empty(Rect { x: 0, y: 0, width: w.max(1), height: h.max(1) });
            let s = state_with_band();
            draw_command_band(&s, area, &mut buf, &mut 0, &mut CommandBandHits::default());
        }
    }

    #[test]
    fn closed_band_is_a_noop() {
        let s = AppState::default();
        assert!(s.overlays.command_band.is_none());
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(hits.headers.is_empty());
    }

    /// SQ-0667: the band no longer draws `panel.border` — there is no frame
    /// left for it to style. Falsifies against the pre-amendment band, which
    /// drew a real box in whatever `panel.border` configured (a custom
    /// `style = "double"` used to paint a "╔" at the top-left corner; there
    /// is no corner left to paint).
    #[test]
    fn band_no_longer_draws_a_border() {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed =
            crate::theme::toml_schema::parse("[panel]\nborder = { style = \"double\" }\n").unwrap();
        let mut s = state_with_band();
        s.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert_ne!(buf.cell((0, 0)).unwrap().symbol(), "╔", "no border corner is drawn anymore");
    }

    /// Resize mode still needs SOME affordance that the band is the live
    /// target now that there's no border left to accent (SQ-0667) — the
    /// whole fill tints with `panel.border:active` instead.
    #[test]
    fn resize_mode_tints_the_whole_fill_with_no_border_left_to_accent() {
        let s_normal = state_with_band();
        let mut buf_normal = Buffer::empty(BAND);
        draw_command_band(&s_normal, BAND, &mut buf_normal, &mut 0, &mut CommandBandHits::default());

        let mut s_resize = state_with_band();
        s_resize.resize_mode = true;
        s_resize.resize_target = crate::state::ResizeTarget::CommandBand;
        let mut buf_resize = Buffer::empty(BAND);
        draw_command_band(&s_resize, BAND, &mut buf_resize, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_normal.cell((0, 0)).unwrap().style(),
            buf_resize.cell((0, 0)).unwrap().style(),
            "resize mode visibly tints the band's fill, with no border left to accent instead"
        );
    }

    #[test]
    fn target_height_clamps_and_respects_the_screen() {
        assert_eq!(band_target_height(false, 40, 8), 0);
        assert_eq!(band_target_height(true, 40, 8), 8);
        assert_eq!(band_target_height(true, 40, 2), MIN_BAND_ROWS);
        assert_eq!(band_target_height(true, 40, 99), MAX_BAND_ROWS);
        // A tiny screen wins over the configured height.
        assert_eq!(band_target_height(true, 9, 8), 5);
    }

    #[test]
    fn height_scales_with_the_slide_fraction() {
        assert_eq!(band_height(8, 0.0), 0);
        assert_eq!(band_height(8, 1.0), 8);
        assert_eq!(band_height(8, 0.5), 4);
    }

    #[test]
    fn arity_parses_the_config_spellings() {
        assert_eq!(Arity::parse("solo"), Some(Arity::Solo));
        assert_eq!(Arity::parse("Object"), Some(Arity::Object));
        assert_eq!(Arity::parse("object_opt"), Some(Arity::ObjectOpt));
        assert_eq!(Arity::parse("object?"), Some(Arity::ObjectOpt));
        assert_eq!(Arity::parse("pair"), Some(Arity::Pair));
        assert_eq!(Arity::parse("nonsense"), None);
    }

    #[test]
    fn every_pair_verb_declares_a_preposition() {
        for v in default_verbs() {
            if v.arity == Arity::Pair {
                assert!(v.prep.is_some(), "pair verb `{}` needs a preposition", v.word);
            }
        }
    }

    /// SQ-0667: the VERB column excludes anything already one click away on
    /// the quick row — showing it twice is redundant. Falsifies against HEAD,
    /// where `items(COL_VERB)` is just the raw verb table.
    #[test]
    fn verb_column_excludes_quick_words() {
        let band = CommandBandState::new(default_verbs(), default_quick());
        let items = band.items(COL_VERB);
        assert!(!items.contains(&"look".to_string()), "`look` is in the default quick row");
        assert!(!items.contains(&"wait".to_string()));
        assert!(!items.contains(&"again".to_string()));
        assert!(!items.contains(&"inventory".to_string()));
        // Direction words compare by the direction they name, not by spelling:
        // the quick row says `n s e w` while the table spells them out, and the
        // compass must still be excluded (it appeared in both places).
        for dir in ["north", "south", "east", "west", "up", "down", "in", "out"] {
            assert!(!items.contains(&dir.to_string()), "`{dir}` is one click away on the quick row");
        }
        assert!(items.contains(&"take".to_string()), "an ordinary verb is unaffected");
    }

    /// Direction equivalence follows the EFFECTIVE quick list too: a custom
    /// quick row without `n` puts `north` back in the VERB column.
    #[test]
    fn a_direction_dropped_from_quick_returns_to_the_verb_column() {
        let band = CommandBandState::new(default_verbs(), vec!["look".to_string()]);
        let items = band.items(COL_VERB);
        assert!(items.contains(&"north".to_string()), "`n` no longer in quick -> `north` returns");
        assert!(!items.contains(&"look".to_string()));
    }

    /// The exclusion is config-aware: it follows the EFFECTIVE `quick` list
    /// (the user's custom one when set), not the built-in row. Removing a
    /// word from a custom `quick` puts it back in the VERB column.
    #[test]
    fn verb_column_exclusion_follows_a_custom_quick_list() {
        let mut band = CommandBandState::new(default_verbs(), vec!["take".to_string()]);
        assert!(!band.items(COL_VERB).contains(&"take".to_string()), "custom quick excludes `take`");
        assert!(band.items(COL_VERB).contains(&"look".to_string()), "…and un-excludes `look`, no longer in quick");

        band.quick.clear();
        assert!(band.items(COL_VERB).contains(&"take".to_string()), "removed from quick -> back in VERB");
    }

    // ── SQ-0677: stacked quick block, dividers, current-column hint ────────────

    /// The word flow now stacks UNDER the rose (SQ-0677) rather than beside
    /// it: every word row's y-offset is at or past `ROSE_ROWS`, and the block
    /// is "as narrow as the widest word row" — its width is the max of the
    /// rose's own width and the widest packed word row, not the two added
    /// together the way a side-by-side layout would need. Falsifies against
    /// reverting to the SQ-0675 side-by-side layout, where the words shared
    /// the rose's rows and the block was `rose + gap + words` wide.
    #[test]
    fn quick_words_stack_under_the_rose_not_beside_it() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert!(layout.has_rose);
        assert_eq!(layout.words_y, ROSE_ROWS, "the word flow starts right under the 3-row rose");
        assert!(!layout.word_rows.is_empty(), "the default quick list has non-compass words");

        let widest_row: u16 = layout
            .word_rows
            .iter()
            .map(|row| {
                row.iter()
                    .map(|&(qi, x_off)| x_off + quick[qi].chars().count() as u16)
                    .max()
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0);
        assert!(widest_row <= ROSE_WIDTH.max(widest_row), "sanity");
        assert_eq!(
            layout.width,
            BLOCK_MARGIN + ROSE_WIDTH.max(widest_row) + BLOCK_MARGIN,
            "block width is margins plus max(rose width, widest word row) — never rose + words added"
        );
    }

    /// The stacked block's total height is the rose's rows plus however many
    /// word rows the flow needs — "as many rows as needed" (SQ-0677), not
    /// capped at a fixed constant the way the pre-amendment side-by-side
    /// block was (always exactly 3, regardless of the word list).
    #[test]
    fn block_height_is_rose_rows_plus_word_rows() {
        let quick = default_quick();
        let layout = quick_block_layout(&quick);
        assert_eq!(layout.height, ROSE_ROWS + layout.word_rows.len() as u16);
        assert!(layout.word_rows.len() > 1, "the default quick list needs more than one word row");
    }

    /// The height interplay (SQ-0677, documented above `WORD_ROW_BUDGET`):
    /// a band shorter than the block's natural height still draws the rose
    /// in full and clips whatever word rows don't fit, rather than shrinking
    /// the rose or refusing to show the block at all. Falsifies against a
    /// block that either panics or silently drops the rose when the band is
    /// short.
    #[test]
    fn a_short_band_clips_word_rows_but_keeps_the_rose() {
        let s = state_with_band();
        // Tall enough to fit the rose+columns, short of the full word flow.
        let area = Rect { x: 0, y: 0, width: 120, height: 4 };
        let mut buf = Buffer::empty(area);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, area, &mut buf, &mut 0, &mut hits);
        let out = dump(&buf);
        assert!(out.contains('N') && out.contains('S'), "the rose still draws in full: {out}");
        // Every registered quick hit stays within the drawn area — no hits
        // for clipped rows.
        assert!(hits.quick.iter().all(|(_, r)| r.y < area.bottom()), "no hits below the band: {:?}", hits.quick);
    }

    /// Single-cell `│` dividers separate the quick block from VERB and every
    /// column from its neighbour (SQ-0677), full band height. Falsifies
    /// against removing the `draw_divider` calls in `draw_command_band`.
    #[test]
    fn dividers_separate_the_block_and_every_column() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);

        // The block-to-VERB divider sits right after the block's own width.
        let layout = quick_block_layout(&s.overlays.command_band.as_ref().unwrap().quick);
        let block_divider_x = BAND.x + layout.width;
        assert_eq!(
            buf.cell((block_divider_x, BAND.y)).unwrap().symbol(),
            "\u{2502}",
            "a divider separates the quick block from VERB"
        );

        // One divider between every pair of adjacent columns.
        assert_eq!(hits.columns.len(), BAND_COLS, "sanity: four columns drew");
        for pair in hits.columns.windows(2) {
            let dx = pair[0].1.right();
            assert_eq!(
                buf.cell((dx, BAND.y)).unwrap().symbol(),
                "\u{2502}",
                "a divider separates column {} from column {}",
                pair[0].0,
                pair[1].0
            );
        }

        // Dividers span the full band height, not just the columns' own rows.
        assert_eq!(
            buf.cell((block_divider_x, BAND.bottom() - 1)).unwrap().symbol(),
            "\u{2502}",
            "the divider reaches the band's bottom row"
        );
    }

    /// A divider cell belongs to neither of its neighbouring columns' hit
    /// rects — clicking exactly on the line between two columns must not be
    /// silently attributed to either one.
    #[test]
    fn divider_cells_are_excluded_from_both_neighbouring_column_rects() {
        let s = state_with_band();
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        for pair in hits.columns.windows(2) {
            let dx = pair[0].1.right();
            assert!(dx < pair[1].1.x, "a gap (the divider) separates {:?} and {:?}", pair[0], pair[1]);
        }
    }

    /// The current column (`band.focus`, Tab/Shift-Tab-driven — SQ-0677)
    /// gets a visible hint: `band.column_header:active` on its header row for
    /// the object/prep columns, whose header moves when Tab moves focus.
    #[test]
    fn the_current_column_header_follows_focus() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("take"); // opens HERE (focus lands there) and CARRIED
        }
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_HERE);
        let mut buf_here = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf_here, &mut 0, &mut CommandBandHits::default());

        s.overlays.command_band.as_mut().unwrap().step_column(1); // -> CARRIED
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_CARRIED);
        let mut buf_carried = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf_carried, &mut 0, &mut CommandBandHits::default());

        assert_ne!(
            buf_here.content(),
            buf_carried.content(),
            "moving the current column with Tab visibly moves the header hint"
        );
    }

    /// The current-column hint decorates chrome, not list rows (user
    /// feedback 2026-08-05, retiring VERB's top-row underline which read as
    /// a mysteriously decorated entry): the dividers flanking the current
    /// column take the `panel.border:active` accent, uniformly for every
    /// column, and NO list row anywhere gains a text decoration from column
    /// focus. Falsifies against the underline scheme and against unstyled
    /// dividers alike.
    #[test]
    fn the_current_columns_flanking_dividers_carry_the_accent_not_any_row() {
        use ratatui::style::Modifier;

        let style_at = |buf: &Buffer, x: u16, y: u16| buf.cell((x, y)).unwrap().style();

        // VERB current (the band's default focus): the divider on VERB's
        // right flank is styled DIFFERENTLY from the far divider between
        // CARRIED and WITH… (which flanks neither side of the current
        // column) — the accent is a visible difference, whatever the theme
        // resolves it to.
        let s = state_with_band();
        assert_eq!(s.overlays.command_band.as_ref().unwrap().focus, COL_VERB);
        let mut buf = Buffer::empty(BAND);
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        let rect_of = |hits: &CommandBandHits, col: usize| {
            hits.columns.iter().find(|(c, _)| *c == col).expect("column drew").1
        };
        let verb = rect_of(&hits, COL_VERB);
        let carried = rect_of(&hits, COL_CARRIED);
        assert_ne!(
            style_at(&buf, verb.right(), verb.y),
            style_at(&buf, carried.right(), carried.y),
            "the divider flanking the current column differs from a far divider"
        );
        // And no list row anywhere is underlined by column focus.
        for x in verb.x..carried.right() {
            for y in BAND.y..BAND.bottom() {
                assert!(
                    !buf.cell((x, y)).unwrap().style().add_modifier.contains(Modifier::UNDERLINED),
                    "no row wears an underline from column focus (cell {x},{y})"
                );
            }
        }

        // Move focus: the accent follows — the VERB-flank divider goes plain
        // and the divider beside the new current column takes the accent.
        let mut s2 = state_with_band();
        s2.overlays.command_band.as_mut().unwrap().pick_word("take");
        let focus = s2.overlays.command_band.as_ref().unwrap().focus;
        assert_ne!(focus, COL_VERB);
        let mut buf2 = Buffer::empty(BAND);
        let mut hits2 = CommandBandHits::default();
        draw_command_band(&s2, BAND, &mut buf2, &mut 0, &mut hits2);
        let cur = rect_of(&hits2, focus);
        assert_ne!(
            style_at(&buf2, cur.x.saturating_sub(1), cur.y),
            style_at(&buf2, carried.right(), cur.y),
            "the accent moved with the current column"
        );
    }
}
