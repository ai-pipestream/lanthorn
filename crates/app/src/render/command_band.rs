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
//! * Nothing auto-submits. A grammatically complete phrase *arms* the phrase
//!   line; Enter (or a click on it) is what sends it.
//!
//! ```text
//! ┌ Command ─────────────────────────────────────────────────────────────────┐
//! │ > unlock iron door with _                                    Enter: send │
//! │  VERB          WHAT — here        WHAT — carried       WITH…             │
//! │   look          window             brass key           ▸brass key        │
//! │  ▸unlock       ▸iron door          lantern              lantern          │
//! │  n s e w · up down · in out · look inventory wait again       one-click  │
//! └──────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! The caller (`main.rs`) sizes `area` from the animated `PanelSlide` fraction,
//! so `area` may be shorter than the band's target height while a slide is in
//! flight — everything here clips to `area`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use super::paneframe::{InsetSegment, PaneGlyphs};
use crate::render::panel::{draw_panel, PanelSpec, PanelStrip};
use crate::state::{AppState, BandFocus};

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
pub const DEFAULT_QUICK: &[&str] = &[
    "n", "s", "e", "w", "up", "down", "in", "out", "look", "inventory", "wait", "again",
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

/// Rows the band occupies when fully open, including its frame: the configured
/// `height`, clamped to [`MIN_BAND_ROWS`]..=[`MAX_BAND_ROWS`] and then to what
/// the screen can actually spare (never more than half of it, and never so much
/// that the story pane is left with nothing). 0 when the band isn't visible.
pub fn band_target_height(visible: bool, full_height: u16, rows: u16) -> u16 {
    if !visible {
        return 0;
    }
    let want = rows.clamp(MIN_BAND_ROWS, MAX_BAND_ROWS);
    // Leave the help row plus at least 3 rows of story pane.
    let hi = full_height.saturating_sub(4);
    want.min(hi)
}

/// Smallest useful band: frame + phrase + headers + one list row + quick row.
pub const MIN_BAND_ROWS: u16 = 5;
/// Largest band the config may ask for.
pub const MAX_BAND_ROWS: u16 = 14;
/// The shipped default band height, in rows (including the frame).
pub const DEFAULT_BAND_ROWS: u16 = 8;

/// The reserved band height in rows: `target_h` scaled by the slide's current
/// `fraction` (0.0 closed .. 1.0 fully open). Mirrors `inventory_dock_height`.
pub fn band_height(target_h: u16, fraction: f64) -> u16 {
    (target_h as f64 * fraction).round() as u16
}

/// Split `content` (the band's inner rect) into `BAND_COLS` column rects.
/// Returns an empty vec when the band is too narrow to give each column a
/// usable width.
pub fn column_rects(content: Rect) -> Vec<Rect> {
    if content.width < (BAND_COLS as u16) * 6 {
        return Vec::new();
    }
    let each = content.width / BAND_COLS as u16;
    (0..BAND_COLS as u16)
        .map(|i| Rect { x: content.x + i * each, y: content.y, width: each, height: content.height })
        .collect()
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
            let here = if loc != 0 { intro.room_objects(loc) } else { Vec::new() };
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
    /// The phrase line (sends when armed).
    pub phrase: Option<Rect>,
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

    if area.width < 8 || area.height < 3 {
        return;
    }

    let theme = &state.colors.theme;
    let base = theme.get("dialog.background").style;

    // Opaque fill first, so panes behind never show through mid-slide.
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    // The frame follows `panel.border`, picking up the `:active` accent while
    // resize mode is targeting the band (SQ-0238's affordance, retargeted).
    let resize_hl =
        state.resize_mode && state.resize_target == crate::state::ResizeTarget::CommandBand;
    let (border_selector, border_color) = if resize_hl {
        ("panel.border:active", theme.get("panel.border:active").style)
    } else {
        ("panel.border", base)
    };
    let spec = PanelSpec {
        area,
        border_selector,
        border_color: Some(border_color),
        border_style: None,
        glyphs: &PaneGlyphs::default(),
        header_on: true,
        strip: Some(PanelStrip {
            segments: &[InsetSegment { text: "Command", active: false }],
            base,
            active: base,
        }),
        body_fill: None,
    };
    let frame = draw_panel(buf, &spec, theme);

    let content = frame.content;
    if content.height == 0 || content.width < 4 {
        return;
    }

    // ── Phrase line (always the top content row) ──────────────────────────────
    let phrase_area = Rect { x: content.x, y: content.y, width: content.width, height: 1 };
    draw_phrase_line(state, band, phrase_area, buf, base);
    hits.phrase = Some(phrase_area);

    if content.height < 2 {
        *vp_out = 0;
        return;
    }

    // ── Quick row (always the bottom content row, when there is one) ──────────
    let has_quick = content.height >= 3 && !band.quick.is_empty();
    if has_quick {
        let quick_area =
            Rect { x: content.x, y: content.bottom() - 1, width: content.width, height: 1 };
        draw_quick_row(band, quick_area, buf, base, theme, &mut hits.quick);
    }

    // ── Columns ──────────────────────────────────────────────────────────────
    let cols_top = content.y + 1;
    let cols_bottom = if has_quick { content.bottom() - 1 } else { content.bottom() };
    if cols_bottom <= cols_top {
        *vp_out = 0;
        return;
    }
    let cols_area = Rect {
        x: content.x,
        y: cols_top,
        width: content.width,
        height: cols_bottom - cols_top,
    };
    let rects = column_rects(cols_area);
    if rects.is_empty() {
        // Too narrow for four columns: show only the active one, full width.
        let active = match band.focus {
            BandFocus::Column(c) => c,
            BandFocus::Phrase => COL_VERB,
        };
        draw_column(state, band, active, cols_area, buf, base, vp_out, hits);
        return;
    }

    *vp_out = 0;
    for (col, rect) in rects.iter().enumerate() {
        hits.columns.push((col, *rect));
        draw_column(state, band, col, *rect, buf, base, vp_out, hits);
    }
}

/// The phrase under construction, plus the send affordance on the right.
fn draw_phrase_line(
    state: &AppState,
    band: &crate::state::CommandBandState,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
) {
    let theme = &state.colors.theme;
    let armed = band.complete();
    let phrase_style = base.patch(
        theme.get(if armed { "band.phrase:armed" } else { "band.phrase" }).style,
    );

    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(phrase_style);
        }
    }

    let text = band.phrase_text();
    // The caret shows where the next token lands; the filter (if any) is shown
    // in place of it, so type-to-filter is visible on the phrase line too.
    let tail = if !band.filter.is_empty() {
        format!("{}_", band.filter)
    } else {
        "_".to_string()
    };
    let line = if text.is_empty() {
        format!("> {tail}")
    } else {
        format!("> {text} {tail}")
    };
    crate::render::draw_str_clipped(buf, area.x, area.y, &line, phrase_style, area);

    // Right-aligned send hint. Only meaningful once the phrase is complete, but
    // the slot is always reserved so the line doesn't jump.
    let hint = if armed { "Enter: send" } else { "" };
    if !hint.is_empty() {
        let w = hint.chars().count() as u16;
        if area.width > w + 4 {
            let x = area.right() - w;
            crate::render::draw_str_clipped(buf, x, area.y, hint, phrase_style, area);
        }
    }
}

/// The one-click quick-action row. Per decision 2 these FILL the phrase; they
/// never fire a turn on their own.
fn draw_quick_row(
    band: &crate::state::CommandBandState,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    theme: &crate::theme::resolve::Theme,
    hits: &mut Vec<(usize, Rect)>,
) {
    let style = base.patch(theme.get("band.quick").style);
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
        crate::render::draw_str_clipped(buf, x, area.y, word, style, area);
        hits.push((i, r));
        x += w + 1;
    }
}

/// One column: its header row plus the (filtered, scrolled) item list.
fn draw_column(
    state: &AppState,
    band: &crate::state::CommandBandState,
    col: usize,
    area: Rect,
    buf: &mut Buffer,
    base: Style,
    vp_out: &mut usize,
    hits: &mut CommandBandHits,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let theme = &state.colors.theme;
    let reachable = band.col_reachable(col);
    let active = !band.story_focused && band.focus == BandFocus::Column(col);

    let header_style = base.patch(
        theme.get(if active { "band.column_header:active" } else { "band.column_header" }).style,
    );
    let header_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    for x in header_area.x..header_area.right() {
        if let Some(cell) = buf.cell_mut((x, header_area.y)) {
            cell.set_symbol(" ").set_style(header_style);
        }
    }
    let label = format!("{}{}", if active { "▸" } else { " " }, band.column_label(col));
    crate::render::draw_str_clipped(buf, header_area.x, header_area.y, &label, header_style, header_area);
    hits.headers.push((col, header_area));

    let list_h = area.height.saturating_sub(1);
    if active {
        *vp_out = list_h as usize;
    }
    if list_h == 0 {
        return;
    }
    let list_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: list_h };

    // A column that is not reachable yet renders dimmed and empty — the grammar
    // is the point, so showing pickable-looking rows there would lie.
    if !reachable {
        return;
    }

    let items = band.filtered_items(col);
    let label_style = base.patch(theme.get("band.group_label").style);
    if items.is_empty() {
        crate::render::draw_str_clipped(
            buf,
            list_area.x,
            list_area.y,
            "(nothing visible)",
            label_style,
            list_area,
        );
        return;
    }

    let scroll = &band.scroll[col];
    let visible = list_area.height as usize;
    let total = items.len();
    let selected = scroll.selected.min(total.saturating_sub(1));
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
        let style = if is_selected && active {
            theme.get("dialog.list_selected").style
        } else {
            base
        };
        let marker = if is_selected && active { "▸" } else { " " };
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
            theme.get("scrollbar").style,
        );
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, CommandBandState};

    const BAND: Rect = Rect { x: 0, y: 0, width: 78, height: 8 };

    fn state_with_band() -> AppState {
        let mut s = AppState::default();
        let mut band = CommandBandState::new(default_verbs(), default_quick());
        band.here = vec!["iron door".to_string(), "mailbox".to_string()];
        band.carried = vec!["brass key".to_string(), "lantern".to_string()];
        band.story_focused = false;
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
        assert!(out.contains("Command"), "title strip");
        assert!(out.contains("look"), "a verb");
        assert!(out.contains("VERB"), "the verb column header");
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

    #[test]
    fn phrase_line_arms_and_shows_the_send_hint() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("look");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        let out = dump(&buf);
        assert!(out.contains("> look"), "phrase line shows the picked verb");
        assert!(out.contains("Enter: send"), "a complete phrase arms the line");
    }

    #[test]
    fn incomplete_phrase_has_no_send_hint() {
        let mut s = state_with_band();
        {
            let b = s.overlays.command_band.as_mut().unwrap();
            b.pick_word("unlock");
        }
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert!(!dump(&buf).contains("Enter: send"), "a pair verb alone is not armed");
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

    #[test]
    fn quick_row_is_hit_testable() {
        let mut buf = Buffer::empty(BAND);
        let s = state_with_band();
        let mut hits = CommandBandHits::default();
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut hits);
        assert!(!hits.quick.is_empty(), "quick words emit hit rects");
        assert!(hits.phrase.is_some(), "the phrase line emits a hit rect");
        assert_eq!(hits.headers.len(), BAND_COLS, "every column header is clickable");
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
        assert!(hits.phrase.is_none());
    }

    #[test]
    fn band_follows_panel_border_style() {
        let scheme = crate::colors::GhosttyScheme::default();
        let parsed =
            crate::theme::toml_schema::parse("[panel]\nborder = { style = \"double\" }\n").unwrap();
        let mut s = state_with_band();
        s.colors.theme = crate::theme::resolve::resolve_theme(&scheme, &parsed);
        let mut buf = Buffer::empty(BAND);
        draw_command_band(&s, BAND, &mut buf, &mut 0, &mut CommandBandHits::default());
        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "╔", "double top-left corner");
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
}
