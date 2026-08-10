//! Story-pane renderer over the engine-neutral [`ScreenModel`] tree.
//!
//! One renderer draws both engines. The **simple** case — a single text-grid
//! over a single text-buffer (the Z-machine shape), or a lone buffer — routes to
//! the existing `draw_upper_window` + `render_transcript` path, so the Z-machine
//! output stays byte-identical. Any richer Glulx tree (multiple/other windows)
//! uses the generic recursive path: `Pair` splits the rect and recurses, `Grid`
//! leaves draw positioned cells, the **primary** `Buffer` leaf draws through the
//! transcript renderer (keeping search / persistence / styling), extra buffers
//! draw their inline content, and `Blank`/graphics leaves are placeholders.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::colors::ColorScheme;
use crate::engine::{BorderPref, BufferWindow, Introspect, PositionedWindow, ScreenModel, StatusModel, WinNode};
use crate::render::transcript::{draw_str_runs, render_transcript, visible_wrapped_lines_kinded};
use crate::render::upper_window::{draw_grid, draw_grid_transparent, draw_upper_window};
use crate::state::{AppState, TranscriptKind};

/// Metrics the story-pane render reports back for scrollbar / mouse routing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryPaneMetrics {
    /// Whether the (primary) transcript drew a scrollbar gutter.
    pub scrollbar: bool,
    /// The largest meaningful `transcript_scroll` value.
    pub max_scroll: u16,
    /// The transcript viewport height (rows).
    pub viewport_rows: u16,
    /// Total wrapped rows of the transcript this frame (for the [more] pager,
    /// which needs the true total even when it fits — SQ-0404).
    pub total_rows: u16,
    /// Per-frame map from rendered cell `(col, row)` → Glk hyperlink value, for
    /// hit-testing a mouse click to its link. Empty when nothing is linked.
    pub links: Vec<((u16, u16), u32)>,
    /// Whether this frame actually laid the transcript out. `false` on frames
    /// whose story pane carries no text surface at all — a v6 full-screen
    /// picture (splash, Zork Zero's map/rebus takeovers). Cross-frame transcript
    /// bookkeeping (the scroll clamp, the [more] pager's row baseline) must skip
    /// such frames: measuring "rows added" against a picture frame's zero total
    /// re-paged the ENTIRE backlog when the normal frame returned (SQ-0578).
    pub transcript_surface: bool,
}

/// Tally `(grids, buffers, others)` leaf windows in the tree. Used only by tests
/// now that [`is_simple`] classifies structurally (SQ-0325).
#[cfg(test)]
fn count_leaves(node: &WinNode) -> (u32, u32, u32) {
    match node {
        WinNode::Grid(_) => (1, 0, 0),
        WinNode::Buffer(_) => (0, 1, 0),
        WinNode::Blank => (0, 0, 1),
        // A Graphics leaf can't use the simple text path — counts as "other",
        // forcing the generic path.
        WinNode::Graphics(_) => (0, 0, 1),
        // A v6 layered composite (Phase 1b) is likewise never the simple text
        // shape — counts as "other". Its own items aren't tallied; nothing
        // reads this test-only helper's counts below leaf granularity.
        WinNode::Layered(_) => (0, 0, 1),
        WinNode::Pair { first, second, .. } => {
            let a = count_leaves(first);
            let b = count_leaves(second);
            (a.0 + b.0, a.1 + b.1, a.2 + b.2)
        }
    }
}

/// True only for the Z-machine shapes the simple grid/transcript path renders
/// byte-identically: a lone buffer or grid, or a grid status band strictly ABOVE
/// the buffer. Every real Glulx layout (nonzero `content_size` extent) renders
/// through the generic tree path instead, so borders and orientation are honoured
/// (SQ-0325). `content_size == (0, 0)` is the Z-machine marker (`session.rs`
/// hardcodes it; `AppGlk` sets a real extent), so a Glulx grid-over-buffer that
/// once matched here — e.g. Counterfeit Monkey — now correctly takes the generic
/// path. That path always draws `model.grid()` as a full-width top band over the
/// transcript, so it is only correct for the one Z-machine orientation.
fn is_simple(model: &ScreenModel) -> bool {
    if model.content_size != (0, 0) {
        return false;
    }
    match &model.root {
        WinNode::Buffer(_) | WinNode::Grid(_) => true,
        WinNode::Pair { vertical: true, first, second, .. } => {
            matches!(**first, WinNode::Grid(_)) && matches!(**second, WinNode::Buffer(_))
        }
        _ => false,
    }
}

/// The game's live input colour (fg/bg) for the input line, or None when
/// colours are off or the game left both channels Default (theme-neutral).
///
/// The pair comes from `ScreenModel.bg`/`fg` — the PANE PAGE — for v1–5, but a
/// v6 story has no pane page (see `session::v6_screen_model`): every window
/// carries its own pair (§8.3) and the model's stays `Default`. So v6 reads the
/// STORY WINDOW's explicit pair instead, the same source the page/ink already
/// use (`v6::story_bg_rgba`/`story_fg_rgba`) — otherwise the typed input falls
/// back to the theme's grey `input_text` on the game's own white page, while the
/// prose beside it (coloured per-run from its `TextAttrs`) is black. Cell-side,
/// so the packed colours resolve through `resolve_zcolour` exactly as the prose
/// runs in `draw_str_runs` do. (SQ-0532 wave-6)
fn game_input_style(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Style> {
    if !state.config.honor_game_colours {
        return None;
    }
    let (fg, bg) = match &model.root {
        WinNode::Layered(items) => {
            let story = crate::render::v6_layout::classify_windows(items).story;
            let (f, b) = crate::render::v6_layout::story_pair_packed(story);
            (crate::state::unpack_zcolour(f), crate::state::unpack_zcolour(b))
        }
        _ => (crate::state::unpack_zcolour(model.fg), crate::state::unpack_zcolour(model.bg)),
    };
    if matches!(fg, zvm::screen::ZColour::Default) && matches!(bg, zvm::screen::ZColour::Default) {
        return None;
    }
    let mut s = ratatui::style::Style::new();
    if !matches!(fg, zvm::screen::ZColour::Default) {
        s = s.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, zvm::screen::ZColour::Default) {
        s = s.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    Some(s)
}

/// The colour scheme to draw the grid (upper/status) window with. When the game
/// has set a page colour scheme (so the story pane is painted with it), the grid's
/// base `upper_window` colour is overridden to those page colours, so a reverse-video
/// status line reverses the GAME's page (e.g. black-on-white → a white-on-black
/// status bar) instead of the app theme — keeping the status bar consistent with the
/// recoloured pane. Borrows the theme unchanged when no game scheme is set. (SQ-0262)
fn grid_scheme<'a>(state: &'a AppState, model: &ScreenModel) -> std::borrow::Cow<'a, ColorScheme> {
    use zvm::screen::ZColour;
    if !state.config.honor_game_colours {
        return std::borrow::Cow::Borrowed(&state.colors);
    }
    let fg = crate::state::unpack_zcolour(model.fg);
    let bg = crate::state::unpack_zcolour(model.bg);
    if matches!(fg, ZColour::Default) && matches!(bg, ZColour::Default) {
        return std::borrow::Cow::Borrowed(&state.colors);
    }
    let mut c = state.colors.clone();
    let mut base = c.theme.get("upper_window").style;
    if !matches!(fg, ZColour::Default) {
        base = base.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        base = base.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    // The border chrome is entirely our own presentation — Glk provides no border
    // styling — so paint the frame in the same page colours as the content, making
    // the whole status area (content + border) one coloured block on the recoloured
    // page rather than a themed frame around a game-coloured interior. (SQ-0267)
    let mut border = c.theme.get("upper_window_border").style;
    if !matches!(fg, ZColour::Default) {
        border = border.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        border = border.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    // SQ-0309: `draw_grid`/`draw_window_separator` read `upper_window` and
    // `upper_window_border` through `c.theme` (the legacy fields are gone), so
    // the override must land in the theme those selectors derive from (their
    // registry parents are the `chrome`/`line` roles with no delta of their
    // own, so seeding just those two roles reproduces `base`/`border` exactly).
    // Other role-derived selectors this Cow's theme could serve (e.g.
    // `hyperlink`, off `accent`) fall back to the terminal-default role rather
    // than the user's real one — narrow, since only the grid/separator draw path
    // reads this Cow's theme, and only while a game page colour is honoured.
    let mut roles = crate::theme::resolve::Roles::terminal_default();
    roles.chrome = base;
    roles.line = border;
    c.theme = crate::theme::resolve::resolve(&roles, &Default::default(), &Default::default(), &Default::default());
    std::borrow::Cow::Owned(c)
}

/// Render the engine's screen into the story-pane `area`, returning scrollbar /
/// scroll metrics for the (primary) transcript.
pub fn render_story_pane(
    model: &ScreenModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
) -> StoryPaneMetrics {
    // Per-frame: the v6 Layered arm republishes this if the game named a page
    // (SQ-0704). Cleared first so a non-v6 frame — or a v6 game that declares
    // nothing — can never inherit the last frame's page.
    state.v6_story_page.set(None);
    // Paint the story-pane background with the game's current background
    // (theme-safe: only the story pane, never the map/chrome; only a concrete,
    // honoured background — Default keeps the theme).
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        if !matches!(bg, zvm::screen::ZColour::Default) {
            let bg_color = crate::render::resolve_zcolour(bg, &state.colors);
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(ratatui::style::Style::new().bg(bg_color));
                    }
                }
            }
        }
    }

    let gi = game_input_style(model, state);

    if is_simple(model) {
        // Byte-identical Z-machine path: the upper grid (if any) over the
        // transcript.
        let mut links: Vec<((u16, u16), u32)> = Vec::new();
        let gc = grid_scheme(state, model);
        let used = match model.grid() {
            Some(grid) => draw_upper_window(grid, char_mode, &gc, area, buf, state.config.honor_game_colours, &mut links),
            None => 0,
        };
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let tarea = reserve_text_margin(tarea, state, margin_style(model, state), buf);
        let (scrollbar, max_scroll, total_rows, mut tlinks) = render_transcript(&model.status, introspect, state, tarea, buf, gi);
        links.append(&mut tlinks);
        return StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: tarea.height, total_rows, links, transcript_surface: true };
    }

    // Generic multi-window path. Grid windows push their hyperlink cells into
    // `grid_links`; the primary buffer's own links ride on its metrics. (SQ-0258)
    //
    // Clamp the composite to gvm's content bounding box: gvm snaps proportional
    // splits to whole cells and leaves a blank margin, so walking the tree into the
    // FULL pane would let the last right-spine leaf balloon to absorb the surplus
    // width. Render into the box and keep the margin blank (SQ-0303).
    let inner = content_bounds(model, area);
    let mut grid_links: Vec<((u16, u16), u32)> = Vec::new();
    let gc = grid_scheme(state, model);
    let metrics = render_node(&model.root, &model.status, char_mode, introspect, state, inner, buf, gi, &mut grid_links, &gc);
    // Keep gvm's snap-margin (the strips of `area` outside `inner`) clean, so no
    // stale cells from a prior frame or the map remain beside the window tree.
    fill_margin(area, inner, model, state, buf);

    // Prune the graphics protocol cache to only the windows still live in the
    // tree, so a closed window's stale cache entry can't be matched by a
    // reopened window reusing the same id (SQ-0174).
    let mut live = std::collections::HashSet::new();
    collect_graphics_ids(&model.root, &mut live);
    {
        let mut gr = state.graphics_render.borrow_mut();
        gr.retain_live(&live);
        // Closing the last graphics window leaves no placement to carry the deletes
        // its uploads need, so hand them a cell of this frame instead (SQ-0637).
        gr.flush_kitty_deletes(area, buf);
    }

    let mut m = metrics.unwrap_or(StoryPaneMetrics {
        scrollbar: false,
        max_scroll: 0,
        viewport_rows: area.height,
        total_rows: 0,
        links: Vec::new(),
        transcript_surface: false,
    });
    m.links.extend(grid_links);
    m
}

/// The sub-rect of the story pane that gvm's window tree actually covers: the
/// top-left corner of `area` sized to `model.content_size`, clamped to `area`.
/// gvm snaps proportional splits to whole cells and leaves a blank margin
/// (SQ-0303); clamping the composite (and the graphics-rect walk, so
/// `dialog_bounds` agrees with what's drawn) to this keeps the margin blank
/// instead of ballooning the last right-spine window. Falls back to the full
/// `area` when `content_size` is `(0, 0)` (the simple/Z-machine paths — no margin).
pub fn content_bounds(model: &ScreenModel, area: Rect) -> Rect {
    // A v6 Layered root is PIXEL content: the raster/hybrid paths scale the
    // native game frame (e.g. Zork0's 320x200 ≈ 40x25 cells) up to fill the
    // pane, so clamping to the cell content_size would pin the whole game to
    // a native-size postage stamp in the corner (the SQ-0303 gvm snap-margin
    // clamp is for cell-fixed window trees only).
    if matches!(model.root, crate::engine::WinNode::Layered(_)) {
        return area;
    }
    let (cw, ch) = model.content_size;
    if cw == 0 || ch == 0 {
        return area;
    }
    Rect::new(area.x, area.y, cw.min(area.width), ch.min(area.height))
}

/// The background style gvm's snap-margin should be painted with: the game's
/// honoured page background when it set a concrete one (matching the story-pane
/// fill at the top of `render_story_pane`), else the theme transcript background
/// (matching `fill`).
fn margin_style(model: &ScreenModel, state: &AppState) -> ratatui::style::Style {
    if state.config.honor_game_colours {
        let bg = crate::state::unpack_zcolour(model.bg);
        if !matches!(bg, zvm::screen::ZColour::Default) {
            return ratatui::style::Style::new().bg(crate::render::resolve_zcolour(bg, &state.colors));
        }
    }
    state.colors.theme.get("transcript").style
}

/// The text-window inner margin actually applied inside `area`, as
/// `(horizontal, vertical)` cells.
///
/// A discovered garglk.ini's `tmarginx`/`tmarginy` wins (highest precedence,
/// runtime-only — never persisted), else the global config default (SQ-0344);
/// either is capped so at least one cell of text survives. Shared by
/// [`reserve_text_margin`] (which inset the transcript by it) and
/// [`story_screen_dims`] (which must report the same width to the story).
fn effective_text_margin(area: Rect, state: &AppState) -> (u16, u16) {
    let ov = state.garglk_overlay.as_ref();
    let want_x = ov.and_then(|o| o.margin_x).unwrap_or(state.config.text_margin_x);
    let want_y = ov.and_then(|o| o.margin_y).unwrap_or(state.config.text_margin_y);
    (
        want_x.min(area.width.saturating_sub(1) / 2),
        want_y.min(area.height.saturating_sub(1) / 2),
    )
}

/// The screen size, in character cells, that the Z-machine should be told the
/// host has — `(rows, cols)` for header bytes $20/$21.
///
/// ZMSD §8.4: the interpreter "may change the exact dimensions whenever it likes
/// but must write the current height (in lines) and width (in characters) into
/// bytes $20 and $21 in the header." So this measures the REAL story pane
/// (`area` is the pane's content rect) instead of reporting a fixed guess.
///
/// The number reported is the region the game's own screen actually gets:
///
/// - the text margin (`text_margin_x`) is subtracted, because that is where the
///   transcript wraps — declaring a wider screen would make a game's centred
///   full-width form sit wider than the prose beside it, the exact mismatch this
///   replaces;
/// - the transcript's one-column scrollbar gutter is subtracted for the same
///   reason (`render_transcript` always reserves it);
/// - the upper window's frame is subtracted, because `draw_grid` draws the grid
///   INSIDE that frame. Without this the declared width would not fit and the
///   game's rightmost columns would be clipped.
///
/// The three together are exactly the chrome that separates the pane from the
/// story's own columns, so the declared width IS the width the upper grid is
/// rendered at AND the width the transcript wraps at — they can no longer drift
/// apart the way a fixed 80 did.
///
/// `virtual_screen_cols`/`virtual_screen_rows` still win when the user pinned
/// them (see the config docs). Returns `None` for a zero-area pane (before the
/// first frame, or while the story pane is hidden) — there is nothing to report.
pub fn story_screen_dims(area: Rect, state: &AppState) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let sides = state.colors.upper_window_border_sides;
    let on = |s: crate::render::paneframe::BorderStyle| {
        u16::from(s != crate::render::paneframe::BorderStyle::None)
    };
    let border_cols = on(sides.left) + on(sides.right);
    let border_rows = on(sides.top) + on(sides.bottom);
    let (mx, _) = effective_text_margin(area, state);
    let gutter = u16::from(area.width >= 2);
    let cols = state
        .config
        .virtual_screen_cols
        .unwrap_or_else(|| (area.width - 2 * mx).saturating_sub(border_cols + gutter));
    let rows = state
        .config
        .virtual_screen_rows
        .unwrap_or_else(|| area.height.saturating_sub(border_rows));
    Some((rows.max(1), cols.max(1)))
}

/// The screen size actually DECLARED to a running story: [`story_screen_dims`]
/// for the pane, floored at the width the story booted believing it had.
///
/// [`story_screen_dims`] measures the pane, and for the *height* that is the
/// whole story — a game re-declares its upper window's height on every layout
/// (`split_window`, ZMSD §8.7.2.1), so it always re-reads the screen it is given.
///
/// The WIDTH it never re-declares: byte $21 is ours alone, and a v4/v5 status
/// routine reads it ONCE, when it lays the bar out, then updates the fields in
/// place at the column numbers it computed back then. Zork 1 (r52) is the
/// reference case — it paints the reverse-video bar at boot and thereafter only
/// `set_cursor`s to the two field columns it derived from the boot width. Narrow
/// the screen under it and those columns fall outside the window, where
/// §8.7.2.3 makes the move illegal; the interpreter drops it, and the digits
/// land wherever the cursor already was — column 1, on top of the room name.
/// That is the garbled status bar of SQ-0679, and no amount of care in the
/// renderer can undo it: by the time we draw, the game has already overwritten
/// its own text.
///
/// So the declared width may GROW to follow a widened pane (SQ-0533 —
/// Sherlock/Trinity, which do re-read $21, gain the columns; every coordinate
/// computed at the old width is still inside the new screen) but never SHRINK
/// below `boot_cols`, the width THIS session actually booted at. In a pane too
/// narrow for that, the story keeps painting the bar it was laid out for and
/// the pane clips the right of it — the same thing every terminal interpreter
/// shows in an 80-column game squeezed into a 60-column window, and a great
/// deal better than a bar with its room name eaten.
///
/// `boot_cols` is [`GameSession::boot_screen_cols`](crate::session::GameSession::boot_screen_cols):
/// [`zvm::screen::DEFAULT_SCREEN_COLS`] (80) for a session booted without a
/// pre-boot pane seed (SQ-0679's original assumption — every v4+ story used to
/// boot at the fixed default), or the real seeded column count (SQ-0680) when
/// one was given. Flooring at a fixed 80 regardless of what the session
/// actually booted at would silently overwrite a correctly narrow pre-boot
/// seed back up to 80 on the very next poll.
///
/// Exempt: v1–3 (no such header fields — §8.4 starts at v4), v6 (its screen is
/// the native pixel frame, scaled into the pane, never measured from it), and a
/// user-pinned `virtual_screen_cols` (explicit intent wins over our floor).
pub fn declared_story_screen_dims(
    area: Rect,
    state: &AppState,
    version: u8,
    boot_cols: u16,
) -> Option<(u16, u16)> {
    let (rows, cols) = story_screen_dims(area, state)?;
    if version < 4 || version == 6 || state.config.virtual_screen_cols.is_some() {
        return Some((rows, cols));
    }
    Some((rows, cols.max(boot_cols)))
}

/// Reserve the configured text-window inner margin (SQ-0345) inside a
/// text-buffer rect: paint the whole rect with `fill` so the reserved band reads
/// as clean padding, then return the inset rect the transcript draws into.
/// `text_margin_x` blank columns are reserved on each side and `text_margin_y`
/// blank rows top and bottom; a margin wider/taller than the rect is capped so at
/// least one cell of text survives. Applies to the text buffer only — the
/// text-grid/upper window is never inset (its cells are game-positioned). Because
/// `render_transcript` publishes its geometry from the rect it receives, insetting
/// here also keeps mouse selection and the copy path aligned (SQ-0197/SQ-0420).
fn reserve_text_margin(area: Rect, state: &AppState, fill: ratatui::style::Style, buf: &mut Buffer) -> Rect {
    let (mx, my) = effective_text_margin(area, state);
    // Publish the applied horizontal margin so the transcript draws its scrollbar
    // flush against the border (in the right margin band) rather than inset with
    // the text (SQ-0345). Set even in the no-op case so a stale value never leaks.
    state.text_margin_applied.set(mx);
    if mx == 0 && my == 0 {
        return area;
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(fill);
            }
        }
    }
    Rect::new(area.x + mx, area.y + my, area.width - 2 * mx, area.height - 2 * my)
}

/// Blank gvm's snap-margin — the L-shaped region of `area` outside `inner` (the
/// full-height strip right of `inner`, plus the strip below `inner` within its
/// columns) — so no stale cells remain beside the clamped window tree (SQ-0303).
fn fill_margin(area: Rect, inner: Rect, model: &ScreenModel, state: &AppState, buf: &mut Buffer) {
    let style = margin_style(model, state);
    let paint = |r: Rect, buf: &mut Buffer| {
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    };
    let right = Rect::new(inner.right(), area.y, area.right().saturating_sub(inner.right()), area.height);
    let bottom = Rect::new(area.x, inner.bottom(), inner.width, area.bottom().saturating_sub(inner.bottom()));
    paint(right, buf);
    paint(bottom, buf);
}

/// Recursively render a tree node into `area`. Returns the primary buffer's
/// metrics when this subtree contains it. Grid-window hyperlink cells are pushed
/// into `links` (the primary buffer's own links ride on its returned metrics).
fn render_node(
    node: &WinNode,
    status: &StatusModel,
    char_mode: bool,
    introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    game_input: Option<ratatui::style::Style>,
    links: &mut Vec<((u16, u16), u32)>,
    grid_colors: &ColorScheme,
) -> Option<StoryPaneMetrics> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    match node {
        WinNode::Pair { vertical, split, border, key_bg, key_fg, first, second } => {
            let b = if *border { 1 } else { 0 };
            let (a1, sep, a2) = split_area_bordered(area, *vertical, split.fixed, b);
            let m1 = render_node(first, status, char_mode, introspect, state, a1, buf, game_input, links, grid_colors);
            // Only rule between two VISIBLE siblings. A border before a collapsed
            // (zero-extent) window — e.g. Counterfeit Monkey's image pane before it
            // shows a letter — would otherwise draw a stray rule with nothing beyond
            // it (SQ-0325). And skip our separator entirely when the game draws its
            // OWN divider as a graphics window adjacent to the gutter (Kerkerkruip),
            // so we don't double the line — matching a pixel interpreter that leaves
            // the border to the game's chrome (SQ-0332).
            let game_divider = edge_touches_painted_graphics(first, *vertical, true)
                || edge_touches_painted_graphics(second, *vertical, false);
            if b > 0 && !a1.is_empty() && !a2.is_empty() && !game_divider {
                draw_window_separator(sep, *vertical, *key_fg, *key_bg, grid_colors, buf);
            }
            let m2 = render_node(second, status, char_mode, introspect, state, a2, buf, game_input, links, grid_colors);
            m1.or(m2)
        }
        WinNode::Grid(g) => {
            let show_cursor = char_mode && g.cursor_active;
            // Game-managed multi-window (generic) path: the game owns the layout
            // and draws its own borders (e.g. Kerkerkruip renders its panel rules
            // as graphics windows), so draw the grid FRAMELESS at its exact rect —
            // no app frame over the game's own separators, and no borrowed rows
            // (SQ-0303). The simple status-line path keeps the app frame via
            // `draw_upper_window`, so the Z-machine / Counterfeit Monkey status
            // bar (SQ-0267) is unaffected.
            let mut frameless = g.clone();
            frameless.border = BorderPref::NoBorder;
            draw_grid(&frameless, frameless.active_rows, frameless.cursor, show_cursor, grid_colors, area, buf, state.config.honor_game_colours, links);
            None
        }
        WinNode::Buffer(b) => {
            if b.primary {
                let area = reserve_text_margin(area, state, state.colors.theme.get("transcript").style, buf);
                let (scrollbar, max_scroll, total_rows, links) =
                    render_transcript(status, introspect, state, area, buf, game_input);
                Some(StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: area.height, total_rows, links, transcript_surface: true })
            } else {
                render_inline_buffer(b, state, area, buf);
                None
            }
        }
        WinNode::Blank => {
            fill(area, buf, &state.colors);
            None
        }
        WinNode::Graphics(gw) => {
            // Solid/thin graphics windows (a game's chrome: panel dividers, colour
            // bars, backgrounds) render directly as cell backgrounds — exact,
            // grid-aligned, and legible even without an image protocol. A detailed
            // canvas falls through to the image protocol (or a plain fill). (SQ-0332)
            if crate::render::graphics::render_graphics_as_cells(gw, area, buf, false) {
                // painted as cells
            } else if let Some(picker) = state.game_picker.as_ref() {
                state.graphics_render.borrow_mut().render(picker, gw, area, state.colors.theme.get("graphics").style, buf);
            } else {
                // No image protocol: approximate the detailed canvas as colour
                // cells rather than blanking it (SQ-0520).
                crate::render::graphics::render_graphics_as_cells(gw, area, buf, true);
            }
            None
        }
        WinNode::Layered(items) => {
            // Phase 1c: with an image protocol, composite the whole v6 pane as one
            // RGBA canvas in the game's NATIVE pixel space (graphics at exact
            // pixel coords, all text rasterized), then draw it scaled to fill the
            // pane. Without a picker, fall through to the Phase 1b cell composite.
            // With an OVERLAY open, likewise fall through: image placements draw
            // above terminal cells in classic protocols, so a menu/dialog under
            // the v6 image would be invisible — the cell fallback keeps the pane
            // readable behind the overlay until it closes.
            state.v6_image_scale.set(1.0);
            // Frameless mode (SQ-0461): deliberately skip the pixel chrome (both
            // the hybrid ring and the raster composite) and fall through to the
            // cell path below — a compact terminal status band over the story as
            // a normal full-pane transcript. A picker still renders inline story
            // pictures there (the primary-Buffer transcript path blits them at
            // native scale, `v6_image_scale` == 1.0 set just above).
            let frameless = state.config.v6_render == crate::config::V6RenderMode::Frameless;
            // A painted MENU screen prints chrome text INSIDE the story window's
            // box, below the status band — Shogun's boot menu paints rows 21–23
            // over its story buffer (rows 21–25). In HYBRID mode such a takeover
            // screen must NOT take the pixel chrome ring: the ring path splits the
            // menu across the raster ring (items mapping above the terminal
            // viewport) and the terminal overlay (items inside it), the exact
            // mixed raster/text defect (SQ-0484). Routing it to the cell path
            // below renders it as one coherent all-text screen, identical to
            // frameless. BOTH conditions matter (SQ-0494): a grid run that is
            // merely deep but sits OUTSIDE the story box is ordinary gameplay
            // chrome — Arthur paints its status bar at row 12 above a story
            // buffer starting at row 13, and classing that as a menu dropped
            // Arthur's whole ring (top image panel + side bars). RASTER mode
            // deliberately keeps its pixel composite for menus (the reverse-video
            // selection block is fixed in `build_chrome_canvas` instead,
            // SQ-0487) — a raster-mode user wants the pixel aesthetic even
            // on menus.
            let hybrid = state.config.v6_render == crate::config::V6RenderMode::Hybrid;
            let story_box = items.iter().find_map(|pw| {
                // The PRIMARY buffer only: a v6 game can publish a second, non-primary
                // prose window (SQ-0585), and taking its rows as the story box made an
                // ordinary split look like a menu takeover.
                matches!(&pw.node, WinNode::Buffer(b) if b.primary).then(|| {
                    let top = pw.y_px / 16;
                    (top, top + pw.h_px.max(1).div_ceil(16), pw.x_px as u32, pw.x_px as u32 + pw.w_px as u32)
                })
            });
            let has_menu = items.iter().any(|pw| {
                matches!(&pw.node, WinNode::Grid(g)
                    if g.px_texts.iter().any(|t| {
                        let row = (t.y.max(1) - 1) / 16;
                        // SQ-0742: the run must be inside the story box on BOTH axes. The
                        // row test alone calls any chrome glyph that merely shares a row
                        // with the story a takeover — and a game whose frame is drawn with
                        // LINE-DRAWING characters rather than reverse-video spaces has one
                        // on every row of the box. Journey under the Amiga profile draws
                        // exactly that: `│` rules at native x 0 / 256 / 632, all outside
                        // its story box (264..632), on every one of its rows. That routed a
                        // perfectly ordinary gameplay screen to the cell path, which draws
                        // the game's 80 columns 1:1 into a pane of any width — the frame
                        // stopped short of the pane edge and the click map (proportional
                        // over the whole pane) no longer matched where anything was drawn.
                        // Under the IBM PC profile the same rules are reverse-video SPACES,
                        // which trim to empty and never tripped the gate, which is why only
                        // the Amiga route showed it.
                        let x0 = t.x.max(1) as u32 - 1;
                        let x1 = x0 + t.text.chars().count().max(1) as u32 * 8;
                        !t.text.trim().is_empty()
                            && row >= STATUS_BAND_ROWS
                            && story_box.is_some_and(|(top, bot, left, right)| {
                                row >= top && row < bot && x1 > left && x0 < right
                            })
                    }))
            });
            // MODAL overlays only (SQ-0587). The fall-through exists because image
            // placements draw above terminal cells in classic protocols, so a
            // menu/dialog over the story pane would be invisible under the v6 image.
            // The room panel and the tidy animation are not that: both live in the MAP
            // pane and never cover the story, and the code already draws exactly this
            // distinction for the input line. Including them here meant an ordinary
            // move — which re-tidies the map and starts its animation — dropped the
            // whole v6 pixel path for the duration, and Arthur's header art vanished
            // with it.
            if !state.any_modal_overlay_open() && !frameless && !(has_menu && hybrid) {
            if let Some(picker) = state.game_picker.as_ref() {
                let (default_fg, default_bg) = v6_host_pair(state);
                use crate::render::v6_layout as v6;
                let native = v6::native_extent(items);
                let layout = v6::classify_windows(items);
                // The native chrome canvas is built per-branch below (SQ-0469):
                // the raster arm skips the build entirely on an unchanged frame.

                // Hybrid mode (Lane H): draw the chrome as a scaled pixel RING
                // around a terminal story viewport, then render the story window as
                // real terminal text (crisp, selectable, scrollable) inside it — the
                // existing primary-Buffer transcript path, with inline images as
                // bands. Needs a story window; without one — or with a full-screen
                // picture takeover, which has no ring to draw (SQ-0570) — fall
                // through to raster.
                if state.config.v6_render == crate::config::V6RenderMode::Hybrid {
                    if let Some(story) = layout.story.filter(|s| !picture_takeover(s, &layout.chrome, layout.story_gfx, native)) {
                        // This frame renders as chrome bands, not the raster
                        // composite — drop the cached composite so a later
                        // fall-through to raster (map/rebus takeover) cannot
                        // flash a stale screen while its encode runs (SQ-0578).
                        state.graphics_render.borrow_mut().invalidate_v6();
                        // Resuming the pixel path after ANY frame that did not use it
                        // (an overlay was up, a menu takeover, a raster frame): the
                        // terminal no longer holds our placements, but every band is a
                        // cache hit and would send nothing. Force them all to re-upload
                        // on this frame (SQ-0587).
                        if state
                            .v6_path_log
                            .borrow()
                            .last()
                            .is_none_or(|(label, _)| label != "hybrid-ring")
                        {
                            state.graphics_render.borrow_mut().invalidate_chrome_bands();
                        }
                        // SQ-0532 wave-5: a game that set its own story page presents
                        // on a FULL page — Zork Zero boots `set_colour(fg=2 black,
                        // bg=9 white)` and the DOS original's white runs edge to edge:
                        // behind the frame art, through the chrome band surrounds, out
                        // into the letterbox margins. Flood the whole pane with it
                        // before the ring and viewport draw over it (the ring's clear
                        // pixels then show the page, not the theme backdrop). Strictly
                        // gated on the story window's EXPLICIT bg, so a game that sets
                        // none — Journey's black picture panel, Arthur, Shogun's
                        // gameplay screen — keeps today's theme backdrop. Gated on
                        // the LIVE `honor_game_colours` too: the model keeps the
                        // pair the game recorded while colours were honored, so a
                        // `/set-game-colours off` mid-game must skip the flood
                        // here, not rely on the window reading `Default`.
                        if state.config.honor_game_colours {
                            if let Some(p) = v6::story_bg_rgba(Some(story), &state.colors) {
                                fill_pane_page(area, p, buf);
                                // Publish it for inline story pictures: their alpha is
                                // ours to resolve, against THIS page (SQ-0704).
                                state.v6_story_page.set(Some((p[0], p[1], p[2])));
                            }
                        }
                        let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &state.colors);
                        // SQ-0704: a chrome window that named its own page paints it
                        // into its unpainted pixels here (ZMSD §8.8.3.2), so the ring
                        // bands ship self-contained instead of leaving the icons'
                        // clear ground for the terminal to colour in (Zork Zero's
                        // room icons came out on an opaque black box). Same live
                        // `honor_game_colours` gate as the pane flood above.
                        // The painted ground goes under the ring's art and glyphs
                        // and before the pages claim the rest (SQ-0706).
                        v6::blit_paint_ground(&mut canvas, state.v6_paint.borrow().as_deref());
                        if state.config.honor_game_colours {
                            v6::fill_window_pages(&mut canvas, &layout.chrome, layout.story, &state.colors);
                            // …and the story window's own page under the pixels the
                            // ring bands ship (SQ-0704, hybrid half). Raster flattens
                            // its whole canvas opaque before shipping; hybrid ships
                            // only these bands, and they overlap the story box — the
                            // sliver under a top banner, and the flanks. A pixel left
                            // transparent there is the TERMINAL's to resolve, which is
                            // why the icons kept coming out on the terminal background
                            // after the chrome half of this fix landed.
                            v6::fill_story_page_clear(&mut canvas, layout.story, &state.colors);
                        } else {
                            // SQ-0716: colours declined, but a window the game has
                            // PAINTED INTO still gets its page — that page is the
                            // ground its own drawing sits on, not a palette
                            // preference. See `fill_painted_window_pages`.
                            v6::fill_painted_window_pages(
                                &mut canvas,
                                &layout.chrome,
                                layout.story,
                                &state.colors,
                                state.v6_paint.borrow().as_deref(),
                            );
                        }
                        let fs = picker.font_size();
                        let cell_px = (fs.width, fs.height);
                        let pane_dev = (
                            area.width as u32 * fs.width.max(1) as u32,
                            area.height as u32 * fs.height.max(1) as u32,
                        );
                        let scale_center = v6::uniform_scale(native, pane_dev);
                        // Publish the letterbox factor so inline story pictures
                        // (drop-caps, room icons) scale to match the chrome ring.
                        // The scale FACTOR is unchanged by the SQ-0505 anchoring
                        // below (only the vertical offset moves), so publish it now.
                        state.v6_image_scale.set(scale_center.s);
                        let gfx = v6::build_graphics_canvas(&layout.chrome, native);
                        let chrome_runs: Vec<&crate::engine::PxText> = layout
                            .chrome
                            .iter()
                            .filter_map(|it| match &it.node {
                                WinNode::Grid(g) => Some(g.px_texts.iter()),
                                _ => None,
                            })
                            .flatten()
                            .collect();
                        // SQ-0505 dynamic hybrid layout: reclaim the letterbox dead
                        // space below the story when the bottom edge is text-only
                        // (Journey's command menu) or empty (Arthur — header art +
                        // side borders, open below). A game whose frame encloses the
                        // story to the native bottom (Zork0) keeps today's centred
                        // letterbox. `slack` is the vertical letterbox margin in
                        // device pixels (zero when the pane is at/below the scaled
                        // native height — nothing to reclaim, degrade to centred).
                        let scaled_h = (native.1 as f32 * scale_center.s).round() as u32;
                        let slack = pane_dev.1.saturating_sub(scaled_h);
                        let plan = hybrid_bottom_plan(story, &gfx, &chrome_runs, native, slack);
                        let reclaim = !matches!(plan, BottomPlan::Letterbox);
                        // Resolve the story scale, the story viewport, and an
                        // optional bottom-anchored menu scale.
                        //   Letterbox → centred (today's behaviour, unchanged).
                        //   Extend    → top-anchor (off_y = 0), story grows to the
                        //               pane bottom; flanks below the side art blank.
                        //   Menu      → top-anchor the story + chrome, bottom-anchor
                        //               the command strip to the pane bottom, story
                        //               fills between at constant width.
                        let top_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: 0 };
                        let (scale, viewport, menu) = match plan {
                            BottomPlan::Letterbox => {
                                let vp = v6::story_viewport_box(Some(story), &scale_center, (area.width, area.height), cell_px);
                                (scale_center, Rect::new(area.x + vp.x, area.y + vp.y, vp.width, vp.height), None)
                            }
                            // Extend (Arthur) and Frame (Zork0/Shogun) top-anchor the
                            // story and grow it to the pane bottom identically; they
                            // differ only in how the flanks below the side art are
                            // treated — Extend blanks them, Frame stretches them (the
                            // reclaim block below branches on the plan).
                            BottomPlan::Extend | BottomPlan::Frame => {
                                let vp = v6::story_viewport_box(Some(story), &top_scale, (area.width, area.height), cell_px);
                                let (x, y) = (area.x + vp.x, area.y + vp.y);
                                (top_scale, Rect::new(x, y, vp.width, area.bottom().saturating_sub(y)), None)
                            }
                            BottomPlan::Menu => {
                                let menu_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: slack };
                                let vp = v6::story_viewport_box(Some(story), &top_scale, (area.width, area.height), cell_px);
                                let (x, y) = (area.x + vp.x, area.y + vp.y);
                                // The menu strip's top cell: the first terminal row that
                                // actually CARRIES a menu run, mapped through the
                                // bottom-anchored menu scale.
                                //
                                // SQ-0548: it used to be the story's native bottom through
                                // that scale, rounded DOWN. The story scale and the menu
                                // scale round independently, so at some pane widths that
                                // floor landed one row ABOVE the first menu row, and the
                                // leftover row entered the menu band carrying no runs.
                                // `decompose_chrome_strips` classes a run-less row `Empty`
                                // and coalesces it into an ART strip, so that row redrew a
                                // squashed slice of the frame's bottom edge full-width
                                // across the pane — on top of the flank panel fill and its
                                // divider. That is the width-dependent dark bar under the
                                // left picture column: at widths where the floor happened
                                // to land on the first menu row there was no leftover row
                                // and no bar, which is why it came and went with the pane.
                                // Anchoring on the runs keeps the story viewport — and with
                                // it the flank fill and divider — reaching the menu at every
                                // width. A run below the story bottom can only map at or
                                // below the old floor, so the viewport never shrinks.
                                let story_bottom = story.y_px as u32 + story.h_px as u32;
                                let menu_top = chrome_runs
                                    .iter()
                                    .filter(|t| !t.text.trim().is_empty() && (t.y.max(1) as u32 - 1) >= story_bottom)
                                    .map(|t| run_cell(t, &menu_scale, cell_px, area).1)
                                    .min()
                                    .map(|r| r.clamp(0, u16::MAX as i32) as u16)
                                    .unwrap_or_else(|| {
                                        let dev = slack as f32 + story_bottom as f32 * scale_center.s;
                                        (dev / cell_px.1.max(1) as f32).floor() as u16
                                    });
                                let menu_top = menu_top.clamp(y + 1, area.bottom());
                                (top_scale, Rect::new(x, y, vp.width, menu_top.saturating_sub(y)), Some(menu_scale))
                            }
                        };
                        // SQ-0582: a status bar the game OVERLAYS on its story window
                        // (advent.z6) leaves no chrome ring to carry it. Reserve its
                        // rows off the top of the story viewport, so the band below
                        // decomposes it exactly like a game that reserved the space
                        // itself — a solid full-width Text strip, with the transcript
                        // starting under it instead of scrolling through it. Measured
                        // from the strip's own runs, not its declared height: a 20px
                        // window is 1.25 cells tall but carries a single text row.
                        let overlay_strip = overlaid_status_strip(&layout.chrome, story, native.0);
                        // The overlaid strip's native bottom, so `decompose_chrome_strips`
                        // counts its runs as band content: they sit INSIDE the story box,
                        // which its above/below test rejects.
                        let overlay_bottom =
                            overlay_strip.map(|s| s.y_px.saturating_add(s.h_px) as i32).unwrap_or(0);
                        let viewport = match overlay_strip {
                            Some(strip) => {
                                let last = match &strip.node {
                                    WinNode::Grid(g) => {
                                        let bound = strip_rows(strip, g);
                                        g.px_texts
                                            .iter()
                                            .filter(|t| !t.text.trim().is_empty())
                                            .filter(|t| {
                                                bound.is_some_and(|b| (t.y.max(1) - 1) / 16 <= b)
                                            })
                                            .map(|t| run_cell(t, &scale, cell_px, area).1)
                                            .max()
                                    }
                                    _ => None,
                                };
                                match last {
                                    Some(r) if r >= area.y as i32 => {
                                        let top = (r as u16).saturating_add(1).min(area.bottom());
                                        let y = viewport.y.max(top);
                                        Rect::new(viewport.x, y, viewport.width, viewport.bottom().saturating_sub(y))
                                    }
                                    _ => viewport,
                                }
                            }
                            None => viewport,
                        };
                        // SQ-0500: a full-width chrome band (top/bottom) is carved
                        // into horizontal strips — an ART strip (opaque frame
                        // graphics behind it) keeps the scaled pixel RING; a
                        // TEXT-ONLY strip (no graphics behind, just status/menu
                        // runs) paints as crisp terminal CELLS. Journey's bottom
                        // command menu becomes text while its left picture column
                        // (a narrow side band) stays ring; Arthur's status row
                        // becomes text between the art panel above and the story
                        // below; Zork0's status sits ON banner art so every strip
                        // stays ring. The graphics-only canvas answers "art behind
                        // this strip?" — the full chrome canvas can't, since its
                        // rasterized text is itself opaque.
                        let bands = v6::chrome_bands(area, viewport);
                        // SQ-0505: in the Menu plan the bottom band IS the command
                        // strip — decompose it through the bottom-anchored `menu`
                        // scale, and the top+side ring bands through the story
                        // `scale`. Each strip is later drawn through the scale it was
                        // classified with, so the menu lands at the pane bottom while
                        // the story/top/sides stay top-anchored.
                        let mut ring_bands = bands;
                        let menu_bands: Vec<Rect> = if menu.is_some() {
                            let vb = viewport.bottom();
                            let m: Vec<Rect> = ring_bands.iter().copied().filter(|b| b.width == area.width && b.y == vb).collect();
                            ring_bands.retain(|b| !(b.width == area.width && b.y == vb));
                            m
                        } else {
                            Vec::new()
                        };
                        // SQ-0511: the Frame/Menu plans STRETCH the side flank bands to
                        // fill the reclaimed space (drawn below); the whole flank band
                        // survives here so the stretch has a full-height target. The
                        // Extend plan (Arthur) instead CLIPS the ring bands to the chrome
                        // art's actual vertical extent (its lowest opaque native row,
                        // mapped through the story scale) so the flanks BELOW its side
                        // art stay the theme backdrop — no art stretching or tiling there.
                        // Letterbox is untouched (its bands lie within the scaled canvas).
                        let stretch_flanks = matches!(plan, BottomPlan::Frame | BottomPlan::Menu);
                        state.v6_ring_plan.set(match plan {
                            BottomPlan::Letterbox => "letterbox",
                            BottomPlan::Extend => "extend",
                            BottomPlan::Frame => "frame",
                            BottomPlan::Menu => "menu",
                        });
                        state.v6_ring_clip.set(None);
                        if reclaim && !stretch_flanks {
                            let ch = cell_px.1.max(1) as f32;
                            let art_bottom_px =
                                (0..gfx.height()).rev().find(|&y| (0..gfx.width()).any(|x| gfx.get_pixel(x, y)[3] >= 128));
                            let clip_row = match art_bottom_px {
                                Some(y) => area.y + ((scale.off_y as f32 + (y + 1) as f32 * scale.s) / ch).ceil() as u16,
                                None => area.y,
                            };
                            // SQ-0571: the clip must never guillotine a chrome TEXT row
                            // that sits between the art and the story — Arthur's status
                            // bar. The clip rounds the art's native bottom UP through the
                            // scale; `run_cell` maps a run's native top by ROUNDing. Both
                            // read the same native boundary (Arthur's art ends at 192, its
                            // status row starts at 192), so whenever `192·s/cell_h` has a
                            // fraction >= 0.5 the two agree and the clip lands exactly ON
                            // the status row, evicting it from the band. With no Text strip
                            // covering it the run is never cleared from the band canvas
                            // (`clear_text_rows` below), so the status painted as a squashed
                            // raster slice of the frame instead of crisp cells — the
                            // width-dependent "corrupted location bar" (broken at 96..=99
                            // columns on an 8x17 cell, clean at 95 and 100).
                            //
                            // Raise the clip past the LAST pure-text chrome row above the
                            // story. Deliberately only text rows: a run-less row below the
                            // art still gets clipped (it would otherwise coalesce into an
                            // Art strip and redraw a squashed slice of the frame's edge,
                            // the SQ-0548 defect), and a run OVER art is already ring
                            // content that the unraised clip places correctly.
                            let story_top = story.y_px as i32;
                            let text_above = chrome_runs
                                .iter()
                                .filter(|t| {
                                    let py = t.y.max(1) as i32 - 1;
                                    !t.text.trim().is_empty()
                                        && py + 16 <= story_top
                                        && !v6::region_has_opaque(
                                            &gfx,
                                            t.x.max(1) as u32 - 1,
                                            py.max(0) as u32,
                                            t.text.chars().count().max(1) as u32 * 8,
                                            16,
                                        )
                                })
                                .map(|t| run_cell(t, &scale, cell_px, area).1)
                                .max();
                            let clip_row = match text_above {
                                Some(r) if r >= 0 => clip_row.max((r as u16).saturating_add(1)),
                                _ => clip_row,
                            };
                            // SQ-0582: never clip above the story viewport. The rule
                            // above only spares text that sits above the story WINDOW,
                            // so a bar the game overlays on the story instead (advent.z6)
                            // matched neither test — with no art either, the clip landed
                            // at the pane top and dropped the very band the inset above
                            // just reserved for it. Whatever is above the viewport is
                            // chrome by construction; it survives.
                            let clip_row = clip_row.max(viewport.y);
                            // Record what clipped the ring (SQ-0587). Arthur's side
                            // borders live in the flank bands, and this clip is what
                            // drops them: it trims the ring to the graphics canvas's
                            // lowest opaque row, so a canvas that lost its lower art
                            // takes the side borders with it.
                            state.v6_ring_clip.set(Some((
                                art_bottom_px.map(|y| y as u16).unwrap_or(u16::MAX),
                                clip_row,
                            )));
                            for b in &mut ring_bands {
                                if b.y >= clip_row {
                                    b.height = 0;
                                } else {
                                    b.height = b.height.min(clip_row - b.y);
                                }
                            }
                            ring_bands.retain(|b| b.height > 0 && b.width > 0);
                        }
                        // SQ-0511: the native row a stretched flank band's art reaches down
                        // to. Frame flanks span the full canvas height (Zork0/Shogun columns
                        // are opaque to the native bottom); the Menu flank (Journey's picture
                        // column + divider) reaches the story bottom, where the bottom-anchored
                        // menu strip begins — so the divider runs unbroken to the menu.
                        let flank_native_bottom = match plan {
                            BottomPlan::Menu => story.y_px as u32 + story.h_px as u32,
                            _ => native.1 as u32,
                        };
                        // Cell rects of the secondary prose windows: the ring leaves
                        // those rows to them (SQ-0585).
                        let panel_rects: Vec<Rect> = layout
                            .chrome
                            .iter()
                            .filter(|pw| matches!(&pw.node, WinNode::Buffer(b) if !b.primary && !b.lines.is_empty()))
                            .map(|pw| px_rect_to_cells(pw, &scale, cell_px, area, 0))
                            .collect();
                        let strips = decompose_chrome_strips(&ring_bands, area, &scale, cell_px, story, overlay_bottom, &panel_rects, &gfx, &chrome_runs);
                        // An ART strip with no actual art behind it draws a rasterized
                        // slice of the chrome canvas — which carries TEXT too, so on a
                        // text-only v6 story (advent) that is pure noise painted over the
                        // pane. Under a graphics protocol the image composites ABOVE the
                        // cells, so it cannot even be overdrawn. Skip those, and let
                        // `/dump-windows` say which ones were skipped. (SQ-0585)
                        let strip_has_art = |r: &Rect| -> bool {
                            let ch = cell_px.1.max(1) as f32;
                            let top = ((r.y.saturating_sub(area.y)) as f32 * ch - scale.off_y as f32)
                                / scale.s.max(0.001);
                            let bot = ((r.bottom().saturating_sub(area.y)) as f32 * ch - scale.off_y as f32)
                                / scale.s.max(0.001);
                            let y0 = top.max(0.0) as u32;
                            let h = (bot.max(0.0) as u32).saturating_sub(y0).max(1);
                            crate::render::v6_layout::region_has_opaque(&gfx, 0, y0, gfx.width(), h)
                        };
                        let menu_strips = match &menu {
                            Some(ms) => decompose_chrome_strips(&menu_bands, area, ms, cell_px, story, overlay_bottom, &panel_rects, &gfx, &chrome_runs),
                            None => Vec::new(),
                        };
                        // SQ-0504: rows drawn as terminal CELLS (pure-text strips)
                        // must not ALSO reach the pixel bands. Carve every text-strip
                        // run's native rows out of the band canvas: excludes the
                        // rasterized menu/status from every uploaded band image (a
                        // sub-cell letterbox boundary otherwise bleeds the raster bar
                        // behind the cells) and decouples each art band's hash from
                        // the menu text (navigating the menu re-encodes only changed
                        // art, not every band). Beside-story runs — Journey's vertical
                        // picture/text divider — are NOT text strips, so they stay in
                        // the side band's ring untouched.
                        let text_run_tops: Vec<u16> = strips
                            .iter()
                            .chain(menu_strips.iter())
                            .flat_map(|s| match s {
                                ChromeStrip::Text(_, runs) => runs.iter().map(|t| t.y.max(1) - 1).collect::<Vec<_>>(),
                                ChromeStrip::Art(_) => Vec::new(),
                            })
                            .collect();
                        v6::clear_text_rows(&mut canvas, &text_run_tops);
                        let base = state.colors.theme.get("upper_window").style;
                        // SQ-0511 fix: in the Menu plan the side flanks are drawn at the
                        // UNIFORM scale (aspect preserved — Journey's left picture column
                        // is NOT vertically stretched); only each flank's full-height
                        // divider/border column is extended down through the reclaimed gap
                        // to the bottom-anchored menu. Compute those narrow extension bands
                        // up front so their cache keys join the live set (else they'd be
                        // pruned and re-encoded every frame). The Frame plan (Zork0/Shogun)
                        // still stretches its whole flank (border art, no story picture).
                        let divider_exts: Vec<(Rect, (u32, u32, u32, u32))> = if matches!(plan, BottomPlan::Menu) {
                            strips
                                .iter()
                                .filter_map(|s| match s {
                                    ChromeStrip::Art(r) if r.width < area.width => flank_divider_extension(
                                        *r, area, viewport, &scale, cell_px, story, native, &canvas, viewport.bottom(),
                                    ),
                                    _ => None,
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        {
                            let mut gr = state.graphics_render.borrow_mut();
                            gr.begin_band_log();
                            let live: std::collections::HashSet<_> = strips
                                .iter()
                                .chain(menu_strips.iter())
                                .filter_map(|s| match s {
                                    ChromeStrip::Art(r) => Some((r.x, r.y, r.width, r.height)),
                                    ChromeStrip::Text(..) => None,
                                })
                                .chain(divider_exts.iter().map(|(r, _)| (r.x, r.y, r.width, r.height)))
                                .collect();
                            gr.retain_chrome_bands(&live);
                            for strip in &strips {
                                if let ChromeStrip::Art(r) = strip {
                                    if !strip_has_art(r) {
                                        continue;
                                    }
                                }
                                match strip {
                                    // SQ-0511: a SIDE flank band (narrower than the pane)
                                    // under the FRAME stretch plan is drawn vertically
                                    // stretched to fill the reclaimed space, keeping the
                                    // horizontal (uniform) scale; the native crop is the
                                    // flank columns from this band's device top (via the
                                    // uniform scale) down to the flank art's bottom. Menu
                                    // flanks and every band under the non-stretch plans draw
                                    // at the uniform scale (aspect preserved).
                                    ChromeStrip::Art(r) => {
                                        // SQ-0547: a Menu-plan SIDE flank is a panel — flood the
                                        // whole column with the game's own panel colour (sampled
                                        // from the art's outer edge) and centre the art in it,
                                        // instead of top-anchoring the art over bare backdrop.
                                        // The divider extension below re-draws over this fill, and
                                        // the frame bands stay wherever their own strips put them.
                                        // The flank's own divider extension, so the art can
                                        // leave a column of panel fill beside it.
                                        let div = divider_exts
                                            .iter()
                                            .map(|(e, _)| *e)
                                            .find(|e| e.x >= r.x && e.x < r.right());
                                        let panel = (matches!(plan, BottomPlan::Menu) && r.width < area.width)
                                            .then(|| menu_flank_panel(*r, viewport, &scale, cell_px, story, native, &gfx, div))
                                            .flatten();
                                        if let Some((bg, dest, crop)) = panel {
                                            fill_pane_page(*r, bg, buf);
                                            gr.draw_chrome_band_stretched(picker, &canvas, dest, crop, buf);
                                        } else if let Some(crop) = (matches!(plan, BottomPlan::Frame) && r.width < area.width)
                                            .then(|| flank_crop(*r, area, &scale, cell_px, flank_native_bottom, native))
                                            .flatten()
                                        {
                                            gr.draw_chrome_band_stretched(picker, &canvas, *r, crop, buf);
                                        } else {
                                            gr.draw_chrome_band(picker, &canvas, &scale, area, *r, buf);
                                        }
                                    }
                                    ChromeStrip::Text(r, runs) => draw_chrome_text_strip(
                                        runs, *r, &scale, cell_px, area, base, state.config.honor_game_colours, &state.colors, buf,
                                    ),
                                }
                            }
                            // SQ-0511 fix: extend each Menu flank's divider/border column
                            // through the reclaimed gap to the menu (a uniform column, so
                            // the vertical replicate is invisible); the rest of the gap is
                            // left undrawn (theme backdrop, matching the flank's own
                            // never-painted background beside the divider).
                            for (ext, crop) in &divider_exts {
                                gr.draw_chrome_band_stretched(picker, &canvas, *ext, *crop, buf);
                            }
                            if let Some(ms) = &menu {
                                for strip in &menu_strips {
                                    match strip {
                                        ChromeStrip::Art(r) => gr.draw_chrome_band(picker, &canvas, ms, area, *r, buf),
                                        ChromeStrip::Text(r, runs) => draw_chrome_text_strip(
                                            runs, *r, ms, cell_px, area, base, state.config.honor_game_colours, &state.colors, buf,
                                        ),
                                    }
                                }
                            }
                            // Record the letterbox geometry for click→game-pixel
                            // mapping (Lane M): the chrome ring shares this scale. In
                            // the Menu plan the interactive read_char region IS the
                            // bottom-anchored command strip, so record ITS scale — a
                            // single V6ClickMap is one linear transform, and the menu
                            // is where clicks are meaningful (story-region clicks map
                            // through the menu offset, but the game reads only the
                            // menu pixels). Extend/Letterbox use one scale everywhere.
                            let click_scale = menu.as_ref().unwrap_or(&scale);
                            // SQ-0550: that scale alone inverts the menu WRONG. The menu
                            // is a TEXT strip, and `draw_chrome_text_strip` packs its game
                            // rows onto CONSECUTIVE terminal rows from the strip's top
                            // (SQ-0543) rather than placing them through the scale — so the
                            // linear inverse drifts by the difference between the two row
                            // pitches, and Journey's player had to click one line below the
                            // command they wanted (two by the bottom row). Hand the map the
                            // strip's row mapping so clicks inside it invert by row index.
                            // The count is the strip's GAME rows, which can be fewer than
                            // its classified height: the classifier places runs through the
                            // scale (leaving gaps the bridge rule absorbs) while the draw
                            // packs them tight, so anything past the last packed row falls
                            // through to the letterbox.
                            let text_rows = menu_strips.iter().find_map(|s| match s {
                                ChromeStrip::Text(r, runs) => {
                                    let rows = runs.iter().map(|t| (t.y.max(1) - 1) / 16);
                                    let first = rows.clone().min()?;
                                    let last = rows.max()?;
                                    Some((r.y, r.height.min(last - first + 1), first))
                                }
                                ChromeStrip::Art(_) => None,
                            });
                            gr.record_hybrid_click_map(area, click_scale, native, cell_px, text_rows);
                        }
                        // The story window as real terminal text (primary-Buffer path).
                        let metrics = render_node(&story.node, status, char_mode, introspect, state, viewport, buf, game_input, links, grid_colors);
                        // SQ-0584: a chrome window the game ERASED more recently than it
                        // printed prose is an opaque panel over the story — advent.z6's
                        // `help` splits window 1 to 160px, erases it and paints a menu
                        // there. Fill its rect over the transcript before the runs below
                        // stamp their glyphs, so the panel reads as a panel instead of
                        // text floating over the room description. `fill` is only set
                        // for a window that is still the newest paint on its own rect,
                        // so an ordinary turn (whose prose is newer) fills nothing.
                        // Record what this frame mapped each window onto, in cells, for
                        // `/dump-windows`. Each entry carries the NATIVE rect it came
                        // from, so the engine can report a window's game-side state and
                        // its terminal placement as one block instead of leaving them to
                        // be correlated by eye (SQ-0585).
                        {
                            use crate::state::V6CellRect;
                            let rec = |label: &str, native: (u16, u16, u16, u16), r: Rect| V6CellRect {
                                label: label.to_string(),
                                native,
                                cells: (r.x, r.y, r.width, r.height),
                            };
                            let mut map = state.v6_cell_map.borrow_mut();
                            map.clear();
                            map.push(rec("path:hybrid-ring", (0, 0, 0, 0), area));
                            state.note_v6_path("hybrid-ring");
                            map.push(rec("pane", (0, 0, 0, 0), area));
                            map.push(rec("viewport", (story.x_px, story.y_px, story.w_px, story.h_px), viewport));
                            map.push(V6CellRect {
                                label: "scale".into(),
                                native: ((scale.s * 100.0) as u16, scale.off_y as u16, cell_px.0, cell_px.1),
                                cells: (0, 0, 0, 0),
                            });
                            for pw in &layout.chrome {
                                let r = px_rect_to_cells(pw, &scale, cell_px, area, 0);
                                let kind = match &pw.node {
                                    WinNode::Buffer(b) if b.primary => "story",
                                    WinNode::Buffer(_) => "panel",
                                    WinNode::Grid(_) => "grid",
                                    WinNode::Graphics(_) => "art",
                                    _ => "?",
                                };
                                map.push(rec(kind, (pw.x_px, pw.y_px, pw.w_px, pw.h_px), r));
                            }
                            for strip in &strips {
                                match strip {
                                    ChromeStrip::Art(r) => {
                                        let label = if strip_has_art(r) {
                                            "strip:art".to_string()
                                        } else {
                                            "strip:art (skipped — no art behind it)".to_string()
                                        };
                                        map.push(rec(&label, (0, 0, 0, 0), *r))
                                    }
                                    ChromeStrip::Text(r, runs) => {
                                        map.push(rec(&format!("strip:text({} runs)", runs.len()), (0, 0, 0, 0), *r))
                                    }
                                }
                            }
                        }
                        // Only windows that START inside the story viewport fill here.
                        // Everything above it belongs to the chrome ring, which draws
                        // its own background — a status strip is flooded by its Text
                        // strip. Letting such a window fill too painted it twice, and
                        // the second rect is the PIXEL-scaled one: advent's 20px bar is
                        // 1.6 terminal rows at a tall pane, so its fill spilled a second
                        // row into the story and the bar read two rows deep (SQ-0582).
                        let fill_chrome: Vec<&PositionedWindow> = layout
                            .chrome
                            .iter()
                            .copied()
                            .filter(|pw| px_rect_to_cells(pw, &scale, cell_px, area, 0).y >= viewport.y)
                            .collect();
                        draw_erase_fills(
                            &fill_chrome, viewport, buf, base, state.config.honor_game_colours, &state.colors,
                            &|pw: &PositionedWindow| px_rect_to_cells(pw, &scale, cell_px, area, 0),
                        );
                        draw_secondary_buffers(&layout.chrome, area, buf, state, &|pw: &PositionedWindow| {
                            px_rect_to_cells(pw, &scale, cell_px, area, 0)
                        });
                        // Chrome text runs that fall INSIDE the story box paint
                        // ON TOP of the terminal transcript (v6 paint order —
                        // Shogun overlays its boot-menu items and selection
                        // caret on the story strip; the ring canvas can't show
                        // them because the terminal viewport covers that area).
                        // Native px → device px (chrome-ring scale) → terminal
                        // cell, glyphs only (no background fill).
                        for it in &layout.chrome {
                            if let WinNode::Grid(g) = &it.node {
                                for t in &g.px_texts {
                                    let px = t.x.max(1) as f32 - 1.0;
                                    let py = t.y.max(1) as f32 - 1.0;
                                    if px < story.x_px as f32
                                        || px >= (story.x_px + story.w_px) as f32
                                        || py < story.y_px as f32
                                        || py >= (story.y_px + story.h_px) as f32
                                    {
                                        continue; // outside the story box → already in the ring
                                    }
                                    let cw = cell_px.0.max(1) as f32;
                                    let ch = cell_px.1.max(1) as f32;
                                    // Pane-relative cell → absolute buffer cell.
                                    let col = area.x as i32 + ((scale.off_x as f32 + px * scale.s) / cw).round() as i32;
                                    let row = area.y as i32 + ((scale.off_y as f32 + py * scale.s) / ch).round() as i32;
                                    if row < viewport.y as i32
                                        || row >= viewport.bottom() as i32
                                        || col < viewport.x as i32
                                        || col >= viewport.right() as i32
                                    {
                                        continue;
                                    }
                                    // Explicit game colours on the run replace the
                                    // theme base per channel; inherited channels
                                    // keep it, reverse toggles (SQ-0488).
                                    let style = v6_run_style(base, t.fg, t.bg, t.style, state.config.honor_game_colours, &state.colors);
                                    let max_w = viewport.right() as usize - col as usize;
                                    if max_w > 0 {
                                        // Untrusted game text (SQ-0639).
                                        let text = crate::render::blank_control_chars(&t.text);
                                        buf.set_stringn(col as u16, row as u16, text.as_ref(), max_w, style);
                                    }
                                }
                            }
                        }
                        return metrics;
                    }
                    // Hint menu open (no streaming story window, SQ-0477): present
                    // the painted screen as positioned terminal text rather than
                    // falling through to the raster composite (an absolutely-
                    // positioned menu rasterizes to an unreadable stamp). The
                    // chrome ring is dropped for this screen — a coherent full-pane
                    // menu. Only when there ARE painted runs; a pure-graphics
                    // no-story frame still falls through to the raster composite.
                    let status_style = state.colors.theme.get("upper_window").style;
                    let runs: Vec<&crate::engine::PxText> = layout
                        .chrome
                        .iter()
                        .filter_map(|it| match &it.node {
                            WinNode::Grid(g) => Some(g.px_texts.iter()),
                            _ => None,
                        })
                        .flatten()
                        .collect();
                    // SQ-0711: this path draws the RUNS and nothing else — every
                    // pixel on the screen is discarded. That is right for a screen
                    // that is only text (Zork Zero's InvisiClues, Shogun's boot
                    // menu: both no-story, both with no painted ground). It is
                    // wrong when the game's picture IS the screen. scopa publishes
                    // no Buffer window at all — its screen is three Grids — and
                    // draws its card table entirely with `erase_window` fills, so
                    // hybrid landed here and rendered SEVEN cells ("abort" and
                    // "OK") out of a 100×34 pane while raster drew the table.
                    // A painted ground means there are pixels that only the raster
                    // composite can show, and it draws the runs over them anyway,
                    // so fall through to it.
                    let painted_ground = state.v6_paint.borrow().is_some();
                    if !painted_ground && runs.iter().any(|t| !t.text.trim().is_empty()) {
                        {
                            // Stamp this path like every other exit (SQ-0637): the
                            // painted menu drops the ring, so the next ring frame is a
                            // RESUME and must re-upload the chrome bands (the SQ-0587
                            // gate reads the last path). Leaving "hybrid-ring" standing
                            // here skipped that re-upload and Zork Zero came back from
                            // its InvisiClues menu with the frame art missing — and
                            // `/dump-windows` reported a ring frame that never ran.
                            let mut map = state.v6_cell_map.borrow_mut();
                            map.clear();
                            state.note_v6_path("painted (hint/menu takeover)");
                            map.push(crate::state::V6CellRect {
                                label: "path:painted (hint/menu takeover)".into(),
                                native: (0, 0, 0, 0),
                                cells: (area.x, area.y, area.width, area.height),
                            });
                        }
                        draw_painted_screen(&runs, 0..u16::MAX, 0, area, buf, status_style, state.config.honor_game_colours, &state.colors, &layout.chrome, native.0);
                        return None;
                    }
                }

                {
                    // Stamp this path too (SQ-0587): otherwise a raster frame leaves
                    // the previous path's record standing and `/dump-windows` reports
                    // the wrong one.
                    let mut map = state.v6_cell_map.borrow_mut();
                    map.clear();
                    state.note_v6_path("raster");
                    map.push(crate::state::V6CellRect {
                        label: "path:raster (full-frame composite)".into(),
                        native: (0, 0, 0, 0),
                        cells: (area.x, area.y, area.width, area.height),
                    });
                }
                // Raster mode (or Hybrid with no story window): rasterize the story
                // text into the clear interior of the native canvas, then draw the
                // whole thing scaled.
                //
                // Generation gate (SQ-0469): the whole canvas rebuild + resize +
                // encode is skipped when nothing that affects the raster changed.
                // `v6_raster_gen` folds every such input into one cheap key; when
                // it matches the last-ready encode we reuse the uploaded protocol
                // and republish the cached scroll metrics — no rebuild, no hash.
                let gen = v6_raster_gen(items, state, area, picker);
                // The game's own page, when it set one (SQ-0532 wave-5). Resolved out
                // here — not inside the gate — because the pane fill below runs on
                // every frame, not just the frames that rebuild the canvas.
                // Gated on the LIVE honor config, like the hybrid flood above: a
                // mid-game `/set-game-colours off` must drop the page even though
                // the model still carries the pair the game set while honored.
                let game_page = if state.config.honor_game_colours {
                    v6::story_bg_rgba(layout.story, &state.colors)
                } else {
                    None
                };
                if state.graphics_render.borrow().v6_wants_build(gen, area) {
                    let (canvas, raster_metrics) = build_v6_raster_canvas(&layout, native, state);
                    // Cache the fresh metrics for skipped frames, then hand the
                    // built canvas to the off-thread resize+encode worker.
                    state.v6_raster_metrics.set(raster_metrics);
                    state.graphics_render.borrow_mut().spawn_v6_encode(picker, canvas, gen, area);
                }
                // SQ-0532 wave-5: the game's own page runs to the pane EDGE. The
                // composite is drawn letterboxed inside the pane, so the margins
                // around it are ordinary terminal cells — with a game-set page they
                // must carry it too, or a white-page game (Zork Zero) floats its
                // white frame on the dark theme backdrop. A game that sets no page
                // (Journey, Arthur, Shogun) — and `honor_game_colours = false`,
                // where the window's bg stays `Default` — keeps the theme backdrop.
                if let Some(p) = game_page {
                    fill_pane_page(area, p, buf);
                }
                // Draw the last-ready encode (this frame's, or the previous one
                // until the worker lands — never blanks to avoid flicker).
                state.graphics_render.borrow_mut().redraw_v6(picker, area, buf);
                // Publish the raster viewport geometry so the shared scroll
                // keybindings, the [more] pager, and mouse routing engage exactly as
                // in the hybrid/terminal paths (SQ-0455). The rasterized text is a
                // scaled pixel image with no cell-accurate transcript grid, so
                // `transcript_geom.area` is the whole pane and mouse mapping is
                // approximate; the scroll/pager math is exact via the returned
                // `StoryPaneMetrics`. Without a story window (`raster_metrics` unset)
                // there is nothing to scroll — fall through to `None`.
                if let Some(rm) = state.v6_raster_metrics.get() {
                    state.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
                        area,
                        first_abs_row: rm.first_visible_row as usize,
                        total_rows: rm.total_rows as usize,
                    }));
                    return Some(StoryPaneMetrics {
                        scrollbar: false,
                        max_scroll: rm.max_scroll,
                        viewport_rows: rm.viewport_rows,
                        total_rows: rm.total_rows,
                        links: Vec::new(),
                        transcript_surface: true,
                    });
                }
                return None;
            }
            } // !any_overlay_open
            // Cell path with a primary story window. Reached three ways: no image
            // protocol (remote/text-only terminals), an overlay is open, or the
            // user chose `v6_render = "frameless"` (SQ-0461) to always present the
            // story this way. The v6 native cell geometry is a
            // 40x25-cell postage stamp on a real terminal and pixel art can't
            // render at all, so render like a classic two-window Z-machine
            // game instead — the status window's text rows across the top of
            // the pane (from the chrome grids' pixel runs, classified into
            // left/center/right anchor groups and laid out as a classic
            // full-width status line — SQ-0467), and the story transcript
            // filling everything below at full size, with working
            // metrics/scrollback. (SQ-0186)
            {
                let layout = crate::render::v6_layout::classify_windows(items);
                let status_style = state.colors.theme.get("upper_window").style;
                // Native screen width in cells (v6 screens vary — Zork0 is
                // 320px/40 cells, others differ) sets the anchor thresholds.
                let (native_w, native_h) = crate::render::v6_layout::native_extent(items);
                let ncols = (native_w as u32).div_ceil(8).max(1);
                // v6 mouse input in the cell path (SQ-0532/A-F4): this branch draws
                // no game image, so there is no letterbox to invert — but the pane
                // still IS the game's screen, so record the proportional pane→native
                // map. Frameless mode lives here permanently; without a map its
                // clicks were dead while the raster/hybrid paths' both worked.
                {
                    let cell_px = state
                        .game_picker
                        .as_ref()
                        .map(|p| {
                            let f = p.font_size();
                            (f.width, f.height)
                        })
                        .unwrap_or((8, 16));
                    state.graphics_render.borrow_mut().record_frameless_click_map(
                        area,
                        (native_w, native_h),
                        cell_px,
                    );
                }
                // Painted text runs across ALL grid windows: the chrome grids
                // carry the status band AND (on a menu/hint screen) the deep
                // absolutely-positioned menu items (Shogun's boot menu paints
                // its three items at native rows 21–23 through window 2).
                let runs: Vec<&crate::engine::PxText> = layout
                    .chrome
                    .iter()
                    .filter_map(|it| match &it.node {
                        WinNode::Grid(g) => Some(g.px_texts.iter()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                if let Some(story) = layout.story {
                    // The frameless pane is composed by RELATION to the story
                    // window, never by an absolute native row (SQ-0549/SQ-0491).
                    // A v6 game puts its chrome wherever its artwork leaves room —
                    // Zork0 and Shogun status at rows 0–1, Arthur's at row 12 under
                    // a 12-row art panel, Journey's command menu at rows 19–24
                    // below a story that starts at row 0 — so the three regions are
                    // defined by where the story box IS:
                    //   above it  → the anchored status band, pinned to the pane TOP
                    //   below it  → the command band, pinned to the pane BOTTOM
                    //   inside it → a painted menu overlay at its own native rows
                    // The story transcript fills whatever is left between them.
                    let story_top = story.y_px / 16;
                    let story_bot =
                        ((story.y_px as u32 + story.h_px as u32).div_ceil(16)).min(u16::MAX as u32) as u16;
                    // The band is MEASURED here and PAINTED below, after the
                    // erase fills (SQ-0712): its rows have to be known to size the
                    // story area, but a window's erase is the ground its own text
                    // is painted on, and the fills go down after the transcript.
                    // Painting the band first put advent's status bar under its own
                    // window's erase — the bar vanished the moment the split stopped
                    // leaving window 0 on top of window 1 and the bar became band
                    // text instead of story-box paint.
                    let top_used = anchored_band_rows(&runs, story_top, area.height);
                    // …and WHERE the transcript starts is the STORY WINDOW'S OWN BOX
                    // (SQ-0697), not "wherever the band happens to end". A game that
                    // parked its story window well down the screen left real empty
                    // screen above it, and that gap is part of the layout it
                    // declared. Shogun's title turns on it: the game prints nine
                    // centred lines across native rows 3–11, then moves window 0 to a
                    // 548x64 box at native row 21 — level with, and left of, the
                    // START/RESTORE/QUIT menu at (235,337) — and prints "You may
                    // choose to:" there. Flush-under-the-band put that prompt on the
                    // line below the banner, nine rows above the menu it belongs
                    // beside, which is exactly what a player reported.
                    //
                    // The gap is measured against the chrome's declared BOX, never
                    // its ink: a chrome window taller than the text in it (Zork Zero's
                    // status panel is 78px of which two rows carry runs) has already
                    // been compressed to its inked rows by the band, and re-counting
                    // its own slack as empty screen would push the transcript down for
                    // art frameless has deliberately dropped. Nothing above the story
                    // at all → nothing to sit below, so the story keeps the pane's top
                    // edge.
                    let chrome_bot = layout
                        .chrome
                        .iter()
                        .filter(|pw| pw.y_px.saturating_add(pw.h_px) <= story.y_px)
                        .map(|pw| ((pw.y_px as u32 + pw.h_px as u32).div_ceil(16)).min(u16::MAX as u32) as u16)
                        .max()
                        .unwrap_or(story_top);
                    let story_row = (top_used + story_top.saturating_sub(chrome_bot))
                        .min(area.height.saturating_sub(1));
                    // The same displacement, as a signed row delta, for everything
                    // else placed at a native row INSIDE the story box: a menu's
                    // glyphs and the erased ground they sit on travel with it.
                    let story_shift = story_row as i32 - story_top as i32;
                    // The command band below the story (Journey's menu): its own
                    // inked native rows, packed against the pane's bottom edge so
                    // it stays locked there at any pane height instead of floating
                    // at its native row over the story text.
                    let below: Vec<u16> = runs
                        .iter()
                        .filter(|t| !t.text.trim().is_empty())
                        .map(|t| (t.y.max(1) - 1) / 16)
                        .filter(|&r| r >= story_bot)
                        .collect();
                    let bottom_span = match (below.iter().min(), below.iter().max()) {
                        (Some(&f), Some(&l)) => Some((f, l - f + 1)),
                        _ => None,
                    };
                    let bottom_used = bottom_span
                        .map(|(_, n)| n)
                        .unwrap_or(0)
                        .min(area.height.saturating_sub(story_row));
                    // A chrome GRAPHICS window entirely BESIDE the story (Journey's
                    // half-screen picture column) is story content, not frame art —
                    // frameless drops the surrounding chrome, but dropping this lost
                    // the illustration the raster and hybrid paths both show. Give it
                    // its native-proportional column and inset the story beside it.
                    // Frame art that spans or overlaps the story (Arthur's header
                    // panel, every game's full-screen backdrop) is NOT beside it and
                    // stays dropped — that is what frameless means.
                    let col_of = |px: u16| (area.width as u32 * px as u32 / native_w.max(1) as u32) as u16;
                    let story_l = story.x_px;
                    let story_r = story.x_px.saturating_add(story.w_px);
                    let sides: Vec<(&&PositionedWindow, bool)> = layout
                        .chrome
                        .iter()
                        .filter(|pw| matches!(&pw.node, WinNode::Graphics(_)))
                        .filter(|pw| pw.y_px < story.y_px.saturating_add(story.h_px) && pw.y_px.saturating_add(pw.h_px) > story.y_px)
                        .filter_map(|pw| {
                            let right_edge = pw.x_px.saturating_add(pw.w_px);
                            if right_edge <= story_l {
                                Some((pw, true))
                            } else if pw.x_px >= story_r {
                                Some((pw, false))
                            } else {
                                None
                            }
                        })
                        // A side column never takes more than half the pane — the
                        // story stays the larger half whatever the game declares.
                        .filter(|(pw, _)| {
                            let w = col_of(pw.x_px.saturating_add(pw.w_px)).saturating_sub(col_of(pw.x_px));
                            w > 0 && w * 2 <= area.width
                        })
                        .collect();
                    let mut story_x = area.x;
                    let mut story_right = area.right();
                    for (_, left) in &sides {
                        if *left {
                            story_x = story_x.max(area.x + col_of(story_l));
                        } else {
                            story_right = story_right.min(area.x + col_of(story_r));
                        }
                    }
                    let mid_y = area.y + story_row;
                    let mid_h = area.height.saturating_sub(story_row + bottom_used);
                    for (pw, _) in &sides {
                        let x = area.x + col_of(pw.x_px);
                        let w = col_of(pw.x_px.saturating_add(pw.w_px)).saturating_sub(col_of(pw.x_px));
                        let rect = Rect::new(x, mid_y, w.min(area.right().saturating_sub(x)), mid_h);
                        render_node(&pw.node, status, char_mode, introspect, state, rect, buf, game_input, links, grid_colors);
                    }
                    let story_area = Rect::new(story_x, mid_y, story_right.saturating_sub(story_x), mid_h);
                    {
                        // Which path drew this frame (SQ-0587): the ring records a full
                        // mapping, so without this a cell-path frame would leave the
                        // last ring frame's numbers in `/dump-windows` and read as if
                        // the ring had run.
                        let mut map = state.v6_cell_map.borrow_mut();
                        map.clear();
                        // Say WHY the ring did not run — "it did not" is half an answer.
                        let why = {
                            let modals = state.open_modal_overlays();
                            if !modals.is_empty() {
                                format!("modal overlay open: {}", modals.join(", "))
                            } else if frameless {
                                "v6_render = frameless".to_string()
                            } else if state.game_picker.is_none() {
                                "no image protocol".to_string()
                            } else if has_menu && hybrid {
                                "painted menu takeover routed here".to_string()
                            } else {
                                "no story window, or a full-screen picture takeover".to_string()
                            }
                        };
                        state.note_v6_path(&format!("cell — {why}"));
                        map.push(crate::state::V6CellRect {
                            label: format!("path:cell — {why}"),
                            native: (0, 0, 0, 0),
                            cells: (story_area.x, story_area.y, story_area.width, story_area.height),
                        });
                    }
                    let m = render_node(&story.node, status, char_mode, introspect, state, story_area, buf, game_input, links, grid_colors);
                    // SQ-0584: erase fields go down over the transcript first — this is
                    // where a painted MENU screen lands (SQ-0484 routes it here out of
                    // hybrid), so without them the menu's text floats over the story it
                    // is supposed to be covering. The cell path is 1:1 with native rows
                    // (8x16 cells), so a window's rect maps by division.
                    draw_erase_fills(
                        &layout.chrome, area, buf, status_style, state.config.honor_game_colours, &state.colors,
                        &|pw: &PositionedWindow| px_rect_to_cells(
                            pw,
                            &crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 },
                            (8, 16),
                            area,
                            story_shift,
                        ),
                    );
                    draw_secondary_buffers(&layout.chrome, area, buf, state, &|pw: &PositionedWindow| {
                        px_rect_to_cells(pw, &crate::render::v6_layout::Scale { s: 1.0, off_x: 0, off_y: 0 }, (8, 16), area, story_shift)
                    });
                    // Chrome text ABOVE the story, as a classic full-width status
                    // line anchored to the pane top. Drawn here, with the rest of the
                    // run stamping and after the erase fills, so a bar sits ON its
                    // own window's erased ground rather than under it (SQ-0712).
                    draw_anchored_status_band(&runs, ncols, story_top, area, buf, status_style, state.config.honor_game_colours, &state.colors);
                    // Painted-screen overlay (SQ-0478): stamp the paint runs INSIDE
                    // the story box as absolutely-positioned terminal text on TOP of
                    // the transcript. A no-op in normal gameplay (chrome grids carry
                    // only the band runs); on a menu screen it draws the items + the
                    // reverse-video selection caret the anchored band drops.
                    draw_painted_screen(&runs, story_top..story_bot, story_shift, area, buf, status_style, state.config.honor_game_colours, &state.colors, &layout.chrome, native_w);
                    if let Some((first, n)) = bottom_span {
                        // Pack the command band's native rows against the pane
                        // bottom: native `first` lands on the first band row.
                        let shift = area.height as i32 - n as i32 - first as i32;
                        draw_painted_screen(&runs, story_bot..u16::MAX, shift, area, buf, status_style, state.config.honor_game_colours, &state.colors, &layout.chrome, native_w);
                    }
                    return m;
                }
                // No streaming story window (a painted menu with win0 in paint
                // mode, or none open): the whole pane IS a painted text screen —
                // stamp every run absolutely rather than falling through to the
                // z-ordered cell composite, which renders the native geometry as
                // an unreadable postage stamp (SQ-0478).
                if runs.iter().any(|t| !t.text.trim().is_empty()) {
                    {
                        // Same stamp rule as the story-window arm above (SQ-0637):
                        // this frame did NOT use the pixel path, so the next ring
                        // frame must be treated as a resume.
                        let mut map = state.v6_cell_map.borrow_mut();
                        map.clear();
                        state.note_v6_path("painted (no story window)");
                        map.push(crate::state::V6CellRect {
                            label: "path:painted (no story window)".into(),
                            native: (0, 0, 0, 0),
                            cells: (area.x, area.y, area.width, area.height),
                        });
                    }
                    draw_painted_screen(&runs, 0..u16::MAX, 0, area, buf, status_style, state.config.honor_game_colours, &state.colors, &layout.chrome, native_w);
                    return None;
                }
            }
            // v6 z-ordered composite (Phase 1b): draw each item in list order —
            // earlier entries (graphics) are background, later entries (text)
            // paint on top. A `Grid` leaf paints only its non-blank cells so an
            // earlier layer shows through the gaps ("cell-text-wins"); other
            // leaves (`Buffer`/`Graphics`) render through the normal recursion.
            let mut result = None;
            for item in items {
                let sub = layered_item_rect(area, item);
                if sub.width == 0 || sub.height == 0 {
                    continue;
                }
                match &item.node {
                    WinNode::Grid(g) => {
                        draw_grid_transparent(g, sub, buf, state.config.honor_game_colours, grid_colors, links);
                    }
                    WinNode::Buffer(_) => {
                        // Transparent composite (cell-text-wins for the buffer, like
                        // draw_grid_transparent for grids): render the transcript into
                        // a scratch buffer, then copy only cells with a visible glyph
                        // onto `buf`, so an earlier graphics layer (a full-screen v6
                        // background window) shows through the empty text areas rather
                        // than being painted over by the buffer's opaque bg fill.
                        let mut scratch = Buffer::empty(sub);
                        let m = render_node(&item.node, status, char_mode, introspect, state, sub, &mut scratch, game_input, links, grid_colors);
                        result = result.or(m);
                        for yy in sub.top()..sub.bottom() {
                            for xx in sub.left()..sub.right() {
                                let visible = scratch
                                    .cell((xx, yy))
                                    .map(|c| { let s = c.symbol(); !s.is_empty() && s != " " })
                                    .unwrap_or(false);
                                if visible {
                                    if let Some(src) = scratch.cell((xx, yy)).cloned() {
                                        if let Some(dst) = buf.cell_mut((xx, yy)) {
                                            *dst = src;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    WinNode::Graphics(gw) => {
                        // v6 background/overlay window: composite per-cell with
                        // transparency (no grey letterbox, empty canvas paints
                        // nothing) so overlapping v6 windows and the text beneath
                        // stay visible.
                        crate::render::graphics::render_graphics_as_cells(gw, sub, buf, true);
                    }
                    _ => {
                        let m = render_node(&item.node, status, char_mode, introspect, state, sub, buf, game_input, links, grid_colors);
                        result = result.or(m);
                    }
                }
            }
            result
        }
    }
}

/// A [`PositionedWindow`]'s absolute cell rect, offset from `area`'s origin and
/// clamped so it never extends past `area`'s bounds (the layered composite's
/// containing rect).
fn layered_item_rect(area: Rect, item: &PositionedWindow) -> Rect {
    let x = area.x.saturating_add(item.x).min(area.right());
    let y = area.y.saturating_add(item.y).min(area.bottom());
    let w = item.w.min(area.right().saturating_sub(x));
    let h = item.h.min(area.bottom().saturating_sub(y));
    Rect::new(x, y, w, h)
}

/// The region a modal dialog should center within: the whole `frame`, minus any
/// Glulx graphics windows.
///
/// Graphics windows are painted through the terminal's own image protocol
/// (kitty/sixel), which draws on top of whatever cells they cover — so a dialog
/// centered over a graphics window is obscured in the real terminal even though
/// it was written into the buffer afterward. This returns the largest rectangle
/// of `frame` that touches no graphics window, so a dialog still spans the story
/// text and the map together where the geometry allows, avoiding only the
/// graphics. `story_area` is where the window tree is laid out (graphics live
/// inside it); pass an empty rect when the story pane isn't shown.
///
/// With no graphics windows this returns `frame` unchanged (today's behavior).
pub fn dialog_bounds(model: &ScreenModel, story_area: Rect, frame: Rect) -> Rect {
    let mut graphics: Vec<Rect> = Vec::new();
    collect_graphics_rects(&model.root, story_area, &mut graphics);
    let mut bounds = frame;
    for g in graphics {
        bounds = subtract_rect(bounds, g);
    }
    bounds
}

/// Walk the tree assigning each leaf its terminal rect (exactly as `render_node`
/// does), collecting every graphics leaf's rect.
fn collect_graphics_rects(node: &WinNode, area: Rect, out: &mut Vec<Rect>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match node {
        WinNode::Pair { vertical, split, border, first, second, .. } => {
            let b = if *border { 1 } else { 0 };
            // Reserve the same separator gutter render_node does, so the graphics
            // rects (and thus `dialog_bounds`) match exactly what's drawn.
            let (a1, _sep, a2) = split_area_bordered(area, *vertical, split.fixed, b);
            collect_graphics_rects(first, a1, out);
            collect_graphics_rects(second, a2, out);
        }
        WinNode::Graphics(_) => out.push(area),
        WinNode::Grid(_) | WinNode::Buffer(_) | WinNode::Blank => {}
        WinNode::Layered(items) => {
            for item in items {
                let sub = layered_item_rect(area, item);
                collect_graphics_rects(&item.node, sub, out);
            }
        }
    }
}

/// Collect the window ids of all live graphics windows in the tree.
///
/// Walks `Layered` too, exactly as [`collect_graphics_rects`] does (SQ-0637). A v6
/// composite IS a `Layered` root, and its graphics leaves reach the protocol path
/// whenever the cell path renders one (a chrome column beside the story, or any
/// frame drawn while a modal overlay is open). Omitting them told
/// [`GraphicsRender::retain_live`] that no window was live, so every such frame
/// cleared the whole cache: a full re-encode each frame, and under kitty a full
/// re-transmit under a NEW id whose predecessors were never deleted.
fn collect_graphics_ids(node: &WinNode, out: &mut std::collections::HashSet<u32>) {
    match node {
        WinNode::Graphics(gw) => {
            out.insert(gw.win);
        }
        WinNode::Pair { first, second, .. } => {
            collect_graphics_ids(first, out);
            collect_graphics_ids(second, out);
        }
        WinNode::Layered(items) => {
            for item in items {
                collect_graphics_ids(&item.node, out);
            }
        }
        WinNode::Grid(_) | WinNode::Buffer(_) | WinNode::Blank => {}
    }
}

/// Remove `g` from `bounds` by a guillotine cut, keeping the largest remaining
/// rectangle. If `g` doesn't overlap `bounds`, `bounds` is returned unchanged.
fn subtract_rect(bounds: Rect, g: Rect) -> Rect {
    let ix = g.x.max(bounds.x);
    let iy = g.y.max(bounds.y);
    let ir = g.right().min(bounds.right());
    let ib = g.bottom().min(bounds.bottom());
    if ix >= ir || iy >= ib {
        return bounds; // no overlap
    }
    // The four rectangles of `bounds` lying outside the overlap band.
    let left = Rect::new(bounds.x, bounds.y, ix.saturating_sub(bounds.x), bounds.height);
    let right = Rect::new(ir, bounds.y, bounds.right().saturating_sub(ir), bounds.height);
    let above = Rect::new(bounds.x, bounds.y, bounds.width, iy.saturating_sub(bounds.y));
    let below = Rect::new(bounds.x, ib, bounds.width, bounds.bottom().saturating_sub(ib));
    [left, right, above, below]
        .into_iter()
        .max_by_key(|r| r.width as u32 * r.height as u32)
        .unwrap_or(bounds)
}

/// Whether the leaf touching a pair's separator gutter is a PAINTED graphics
/// window (the game's own drawn divider). `vertical` is the PARENT pair's split
/// orientation; `high` is true when the gutter lies on this node's high-coordinate
/// edge (i.e. this is the pair's `first` child, whose far edge abuts the gutter).
///
/// Walks structurally: along the same split axis only the child on the gutter side
/// touches it; across axes both children span the parent's edge, so either can. Used
/// to suppress our redundant separator when a game (Kerkerkruip) draws its own
/// graphics-window rule there (SQ-0332) — but only when that window is actually
/// painted, so a game's empty frame windows (narco) still get our rule (SQ-0340).
fn edge_touches_painted_graphics(node: &WinNode, vertical: bool, high: bool) -> bool {
    match node {
        // Only a PAINTED graphics window counts as the game's own divider. A
        // window the game opened but never drew into (narco frames its story with
        // empty graphics windows) is NOT a divider — suppressing our separator
        // there would leave the pane with no visible boundary at all. (SQ-0340)
        WinNode::Graphics(g) => g.canvas.pixels().any(|p| p[3] >= 128),
        // A v6 layered composite (Phase 1b) only ever appears as a whole-tree
        // root (built directly by the v6 adapter, never nested inside a Pair
        // sibling), so it can't be the game's own divider here — treat it like
        // the other non-Pair, non-Graphics leaves.
        WinNode::Buffer(_) | WinNode::Grid(_) | WinNode::Blank | WinNode::Layered(_) => false,
        WinNode::Pair { vertical: v, first, second, .. } => {
            if *v == vertical {
                let child = if high { second } else { first };
                edge_touches_painted_graphics(child, vertical, high)
            } else {
                edge_touches_painted_graphics(first, vertical, high)
                    || edge_touches_painted_graphics(second, vertical, high)
            }
        }
    }
}

/// Split `area` for a pair, reserving `border` cells (0 or 1) between the children
/// for the separator rule. `first` gets `fixed` cells; the separator gets `border`;
/// `second` gets the rest. gvm already reserved this 1-cell gutter between bordered
/// siblings, so the two child areas never include it — the rule is drawn in `sep`.
fn split_area_bordered(area: Rect, vertical: bool, fixed: u16, border: u16) -> (Rect, Rect, Rect) {
    if vertical {
        let f = fixed.min(area.height);
        let b = border.min(area.height - f);
        let first = Rect::new(area.x, area.y, area.width, f);
        let sep = Rect::new(area.x, area.y + f, area.width, b);
        let second = Rect::new(area.x, area.y + f + b, area.width, area.height - f - b);
        (first, sep, second)
    } else {
        let f = fixed.min(area.width);
        let b = border.min(area.width - f);
        let first = Rect::new(area.x, area.y, f, area.height);
        let sep = Rect::new(area.x + f, area.y, b, area.height);
        let second = Rect::new(area.x + f + b, area.y, area.width - f - b, area.height);
        (first, sep, second)
    }
}

/// Fill every cell of the separator gutter `area` with the Glk inter-window
/// rule: a horizontal `─` for a stacked/vertical pair (rule runs across the top
/// child's bottom edge), a vertical `│` for a side-by-side/horizontal pair.
///
/// Glk provides no border styling, so this reuses the existing themeable
/// window-border presentation rather than a dedicated selector: the rule is drawn
/// in `colors.theme.get("upper_window_border")` (the same style the status frame
/// uses), and a user-set glyph override from `colors.upper_window_border_glyphs` — `.top` for a
/// horizontal rule, `.left` for a vertical one — is honoured, else the box-drawing
/// defaults. (A dedicated `window-border` selector can follow when the deferred
/// style redesign lands — do NOT add a new selector here.)
fn draw_window_separator(area: Rect, vertical: bool, key_fg: Option<u32>, key_bg: Option<u32>, colors: &ColorScheme, buf: &mut Buffer) {
    let g = &colors.upper_window_border_glyphs;
    let glyph = if vertical {
        g.top.as_deref().unwrap_or("\u{2500}") // ─
    } else {
        g.left.as_deref().unwrap_or("\u{2502}") // │
    };
    // The separator adopts the split's KEY (new) window colour (SQ-0325 follow-up):
    // draw the rule glyph in `key_fg` on `key_bg` when the game set them, falling
    // back to the themed `upper_window_border` fg/bg per channel when `None`.
    let mut style = colors.theme.get("upper_window_border").style;
    if let Some(rgb) = key_fg {
        style = style.fg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), colors));
    }
    if let Some(rgb) = key_bg {
        style = style.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), colors));
    }
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(glyph).set_style(style);
            }
        }
    }
}

/// Draw an inline (non-primary) buffer window's wrapped, styled lines.
fn render_inline_buffer(b: &BufferWindow, state: &AppState, area: Rect, buf: &mut Buffer) {
    // This window's own Normal-style background (Glulx window colour, SQ-0328)
    // replaces the theme transcript bg when the game set one; `None` keeps the
    // theme background (today's behaviour).
    let base = match (b.panel, b.bg) {
        // A game-set window colour always wins.
        (_, Some(rgb)) => state.colors.theme.get("transcript").style.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), &state.colors)),
        // A chrome panel (Scott room panel) uses the themed `room_panel` colour so
        // the split's top and bottom read as distinct regions.
        (true, None) => state.colors.theme.get("room_panel").style,
        (false, None) => state.colors.theme.get("transcript").style,
    };
    fill_style(area, buf, base);
    if b.lines.is_empty() {
        return;
    }
    let kinds = vec![TranscriptKind::Story; b.lines.len()];
    let styles = vec![base; b.lines.len()];
    // Inline images render as bands only when a game picker exists (same as the
    // transcript); `char_px` is that picker's cell pixel size for pixel-accurate
    // fit. Mirrors `render_middle`.
    let images_enabled = state.game_picker.is_some();
    let char_px = state
        .game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width, f.height)
        })
        .unwrap_or((1, 1));
    let (rows, _total, _first) = visible_wrapped_lines_kinded(
        &b.lines,
        &kinds,
        &styles,
        &b.runs,
        &b.para,
        &b.images,
        char_px,
        images_enabled,
        area.height as usize,
        b.scroll,
        area.width,
        None,
    );
    for (i, wr) in rows.iter().enumerate() {
        let row_y = area.y + i as u16;
        // Inline-image band row: blit the strip for this row instead of text
        // (same branch as the transcript draw loop, Task 8).
        if crate::render::inline_image::try_blit_band_row(state, wr, area.x, area.width, row_y, buf) {
            continue;
        }
        draw_str_runs(buf, area.x, row_y, &wr.text, wr.style, &wr.runs, None, area, &state.colors, state.config.honor_game_colours);
    }
}

/// Fill `area` with the transcript background style.
fn fill(area: Rect, buf: &mut Buffer, colors: &crate::colors::ColorScheme) {
    fill_style(area, buf, colors.theme.get("transcript").style);
}

/// Fill `area` with an explicit `style` (used for a per-window background override).
fn fill_style(area: Rect, buf: &mut Buffer, style: ratatui::style::Style) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// Resolve a themed style's colour to an opaque RGBA for the pixel canvas.
fn style_fg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.fg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Resolve a themed style's background colour to an opaque RGBA for the pixel
/// canvas (the `default_bg` fallback for chrome reverse-video and the story
/// background fill — see [`crate::render::v6_layout::build_chrome_canvas`]).
fn style_bg_rgba(style: ratatui::style::Style, fallback: image::Rgba<u8>) -> image::Rgba<u8> {
    match style.bg {
        Some(ratatui::style::Color::Rgb(r, g, b)) => image::Rgba([r, g, b, 255]),
        _ => fallback,
    }
}

/// Hardcoded v6 raster default ink (light grey) and page (black), used only when
/// neither the theme nor the terminal (OSC 10/11 probe) supplies a concrete RGB.
const RASTER_FALLBACK_INK: image::Rgba<u8> = image::Rgba([220, 220, 220, 255]);
const RASTER_FALLBACK_PAGE: image::Rgba<u8> = image::Rgba([0, 0, 0, 255]);

/// A style channel's concrete RGB, or `None` when it is unset / a terminal-default
/// or named (non-`Rgb`) colour. The pixel canvas needs real bytes, so only an
/// explicit `Color::Rgb` counts as "the theme supplied this channel".
fn style_rgb(c: Option<ratatui::style::Color>) -> Option<image::Rgba<u8>> {
    match c {
        Some(ratatui::style::Color::Rgb(r, g, b)) => Some(image::Rgba([r, g, b, 255])),
        _ => None,
    }
}

/// Resolve the v6 raster canvas's default ink+page as a matched PAIR from a
/// SINGLE source (SQ-0510, reopened). Resolving the two channels independently
/// paired a theme's ink — a cream/beige foreground authored for the theme's own
/// dark page — with a page from a *different* source (the OSC-probed terminal, or
/// the transparent-canvas-over-white compositor backdrop): two colours never
/// designed to sit together, e.g. beige-on-white. So ink and page are drawn from
/// ONE layer, in order:
///   1. Theme — only when the transcript style supplies BOTH a concrete fg and bg RGB.
///   2. OSC terminal colours — only when the probe answered BOTH channels.
///   3. The hardcoded fallback pair (light grey ink on black page).
///
/// A partial layer (theme fg but no bg; only one OSC channel) is skipped whole
/// rather than mixed, so the returned pair is always internally consistent.
fn v6_default_pair(
    themed: ratatui::style::Style,
    osc_fg: Option<image::Rgba<u8>>,
    osc_bg: Option<image::Rgba<u8>>,
) -> (image::Rgba<u8>, image::Rgba<u8>) {
    if let (Some(fg), Some(bg)) = (style_rgb(themed.fg), style_rgb(themed.bg)) {
        return (fg, bg);
    }
    if let (Some(fg), Some(bg)) = (osc_fg, osc_bg) {
        return (fg, bg);
    }
    (RASTER_FALLBACK_INK, RASTER_FALLBACK_PAGE)
}

/// The HOST's resolved default `(ink, page)` pair for the v6 pixel paths — the
/// transcript theme layered over the OSC probe over the fallback, via
/// [`v6_default_pair`]. One function so the hybrid ring, the raster canvas and
/// the tests can never resolve it differently.
pub fn v6_host_pair(state: &AppState) -> (image::Rgba<u8>, image::Rgba<u8>) {
    v6_default_pair(
        state.colors.theme.get("transcript").style,
        state.term_default_colors.fg,
        state.term_default_colors.bg,
    )
}

/// Build the v6 RASTER composite for one frame, in the game's native pixel space:
/// the chrome art, the story page, the wrapped story text in the game's own ink,
/// its inline floats (drop-caps, room icons), the `[more]` prompt — then flattened
/// onto the page so the shipped image is self-contained. Returns the canvas and
/// the story scroll/pager metrics (`None` when there is no story window).
///
/// Public so a test can assert on the EXACT pixels the render composites (a glyph's
/// ink, the page beneath it, a drop-cap's art) instead of re-implementing the
/// pipeline and pinning the re-implementation. (SQ-0532 wave-5)
pub fn build_v6_raster_canvas(
    layout: &crate::render::v6_layout::V6Layout<'_>,
    native: (u16, u16),
    state: &AppState,
) -> (image::RgbaImage, Option<RasterMetrics>) {
    use crate::render::v6_layout as v6;
    let (default_fg, default_bg) = v6_host_pair(state);
    // The story PAIR (SQ-0510, extended in SQ-0532 wave-5): a game-set
    // story-window colour (`set_colour`) wins per channel, else the paired
    // host default. Zork Zero boots `set_colour(fg=2, bg=9)`, so taking its
    // white page while rasterizing the prose in the host's own light default
    // ink drew white-on-white — unreadable. The window's explicit fg wins over
    // `default_fg` exactly as its bg wins over `default_bg`. Both are gated on
    // the LIVE honor config: a mid-game `/set-game-colours off` leaves the
    // recorded pair in the model, and the composite must fall back to the host
    // pair rather than keep painting the game's page/ink.
    let honor = state.config.honor_game_colours;
    let game_page = if honor { v6::story_bg_rgba(layout.story, &state.colors) } else { None };
    let game_ink = if honor { v6::story_fg_rgba(layout.story, &state.colors) } else { None };
    let page = game_page.unwrap_or(default_bg);
    let ink = game_ink.unwrap_or(default_fg);
    let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &state.colors);
    // …and the lines of any SECONDARY prose window (SQ-0729), which the chrome
    // canvas does not draw. The story page below spares them like any chrome text.
    v6::draw_secondary_prose(&mut canvas, &layout.chrome, ink, honor, &state.colors);
    // SQ-0704: each chrome window's own page (ZMSD §8.8.3.2) fills its unpainted
    // pixels before the story is stamped — the story box itself is skipped (see
    // `fill_window_pages`), so `story_clear_native` below still finds it clear.
    // The game's own painted ground — erase_window fills (SQ-0706) — goes UNDER
    // the art and glyphs already on the canvas and BEFORE the window pages claim
    // what is left, because a fill is the oldest thing on the screen: the game
    // filled its rectangle, then printed the label on top of it.
    let grounds = |c: &mut image::RgbaImage| {
        v6::blit_paint_ground(c, state.v6_paint.borrow().as_deref());
        if honor {
            v6::fill_window_pages(c, &layout.chrome, layout.story, &state.colors);
        } else {
            // SQ-0716: colours declined, but a window the game has PAINTED INTO still
            // gets its page — scopa's felt table is a full-screen `erase_window` in
            // explicit green that `drain_erase_fills` drops as a screen clear, so
            // window 1's background is the only surviving record of that drawing.
            // Gating it left a black table under the game's own green stripes and
            // cards. See `fill_painted_window_pages`.
            v6::fill_painted_window_pages(
                c,
                &layout.chrome,
                layout.story,
                &state.colors,
                state.v6_paint.borrow().as_deref(),
            );
        }
    };
    grounds(&mut canvas);
    // What the story box is measured against (SQ-0728): the same layers, MINUS the
    // chrome text. `story_clear_native` shrinks the story window edge by edge until
    // no edge touches an opaque pixel, and its purpose is to seat the prose inside
    // bordering frame ART — Zork Zero's ring, Arthur's plate, Journey's picture
    // panel. Rasterized glyphs are opaque too, so a chrome window the game paints
    // INSIDE window 0 was eating the story box instead of coexisting with it, which
    // is not what a real interpreter does: Shogun's title prints "You may choose
    // to:" at x=47 while its menu window prints "START the game" at x=235, on the
    // same rows. Measured against the full canvas Shogun's declared 548x64 box came
    // back as 548x16 — one row, which `build_main_text` then reports as ZERO visible
    // rows — and Journey's 392x304 text panel came back 392x0. Against the art it is
    // the box each game declared. Same lesson as `build_graphics_canvas` on the
    // hybrid side (SQ-0500): "opaque" is not "artwork".
    let mut obstruction = v6::build_graphics_canvas(&layout.chrome, native);
    grounds(&mut obstruction);
    let mut raster_metrics: Option<RasterMetrics> = None;
    // SQ-0578: only stamp the story when its clear interior can hold at least
    // one full 8x16 text cell. A full-screen picture (Zork Zero's rebus) grows
    // window 0 over the whole screen and paints art across virtually all of it;
    // the inset then leaves a degenerate sliver (the rebus leaves 0x80), and the
    // `.max(1)` below pinned that to a ONE-COLUMN story box — the whole
    // transcript re-wrapped a character per line with a [more] prompt that took
    // hundreds of keypresses to drain. No cell fits → the picture owns the
    // screen: ship the art alone and report no scroll metrics, exactly like the
    // no-story-window case.
    let story_clear =
        v6::story_clear_native(layout.story, &obstruction).filter(|&(_, _, w, h)| w >= 8 && h >= 16);
    if let Some((sx, sy, sw, sh)) = story_clear {
        // Paint the story page opaque (SQ-0510, reopened). Leaving it
        // transparent let whoever composites the image pick the colour
        // instead of us. `story_clear_native` has already inset past any
        // bordering frame art, and `flatten_onto_page` below covers the
        // degenerate case where that inset leaves nothing; inline-image
        // floats redraw on top in `draw_story_text`. So no artwork is covered.
        // The chrome TEXT the game printed inside the box is spared (SQ-0728):
        // window 0's page is under it, not over it — Shogun's title prints its
        // menu into window 0's box and both belong on the screen.
        v6::fill_story_page_under_chrome_text(
            &mut canvas,
            (sx, sy, sw, sh),
            page,
            &layout.chrome,
            state.v6_paint.borrow().as_deref(),
        );
        // …then the story window's OWN absolutely-placed artwork, before any
        // prose: Arthur's intro centres a 584×392 plate in window 0, so the plate
        // is the page's backdrop, not part of the frame ring — and the page fill
        // just above would otherwise wipe it. The probe for the clear interior ran
        // BEFORE this blit, so the text box is still measured against the frame
        // art alone. (SQ-0695)
        v6::blit_story_gfx(&mut canvas, layout.story_gfx);
        // A story window whose own art ENCLOSES it is a CANVAS, not a page
        // (SQ-0729): what it shows is the runs sitting on it, at the coordinates
        // the game's own `set_cursor` named, and a scrolling re-render of
        // everything it ever printed is the wrong reading of the window. So its
        // live runs are painted and there is no transcript on this frame — no
        // prose box, and no scroll metrics, exactly as when a plate owns the
        // screen. See `story_window_is_a_canvas`: fmvpoker alone.
        if story_window_is_a_canvas(layout, native) {
            v6::draw_story_canvas_runs(&mut canvas, layout.story, ink, page, honor, &state.colors);
            return finish_v6_raster_canvas(canvas, page, raster_metrics);
        }
        // Whether any prose belongs on THIS frame, and where (SQ-0707). An
        // absolutely-placed plate is drawn INSTEAD of prose, not under it: the
        // game erases, draws, and waits, so the narration is its own picture-less
        // screen. `None` = the plate owns the screen, and rasterizing scrollback
        // onto it would paint the PREVIOUS screen's text across the art.
        let Some((tx, ty, tw, th)) = v6::story_prose_box((sx, sy, sw, sh), layout.story_gfx) else {
            return finish_v6_raster_canvas(canvas, page, raster_metrics);
        };
        // Window-0 inline pictures (drop-caps, room icons) arrive as
        // transcript-anchored floats (`transcript_images` sidecar):
        // build_main_text wraps text beside them and draw_story_text
        // blits each at its anchored row — they scroll with the text.
        // Non-square 8×16 v6 cell (SQ-0479): columns divide the
        // clear width by FONT_W(8), rows the height by FONT_H(16).
        let (sx, sy) = (tx, ty);
        let cols = (tw / 8).max(1) as u16;
        let rows = (th / 16).max(1) as u16;
        let (main, rm) = build_main_text(state, cols, rows);
        // …sparing the cells another window's own text already holds (SQ-0729).
        // The page fill above spares them; the GLYPHS did not, so the transcript
        // was drawn straight through them. fmvpoker's dealt hand is the report:
        // its five cards fill the frame's interior, window 0's clear rectangle
        // drops onto the box the game gave its bottom prose window, and the boot
        // banner landed on top of "You draw (a) an Eight, (b) a Three, …" — the
        // line the player needs in order to see their draw. The transcript is the
        // host's re-render of window 0's whole history; the label is on the screen
        // now, so the label wins.
        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, ink, &v6::chrome_text_rects(&layout.chrome));
        // [more] pager indicator (SQ-0455): when a single turn's output
        // overflowed the story box the shared pager (SQ-0404) parks the
        // scroll and shows a `[more]` prompt. The raster path can't reserve
        // a terminal row, so draw the prompt as a text run bottom-right of
        // the story box, themed via the `more_prompt` selector (drawn as a
        // reverse-video block, matching the terminal bar).
        if state.pager.active {
            let mp = state.colors.theme.get("more_prompt").style;
            // Reverse-video against whatever the story page/ink actually ARE.
            // When the game set its own pair (Zork Zero's black on white) the
            // prompt must reverse THAT pair, or a themed block resolved from an
            // unrelated source lands as (say) white on white on the game's page.
            // With no game pair the themed `more_prompt` selector still governs,
            // resolved as a PAIR from one source (theme both / OSC both /
            // fallback) so the block and its ink never mix sources.
            let (block, prompt_ink) = match (game_page, game_ink) {
                (Some(p), Some(i)) => (i, p),
                _ => v6_default_pair(mp, state.term_default_colors.fg, state.term_default_colors.bg),
            };
            let label = "[more]";
            let n = label.chars().count() as u32;
            let last_row = rows.saturating_sub(1) as u32;
            let start_col = (cols as u32).saturating_sub(n);
            for (i, ch) in label.chars().enumerate() {
                // 8×16 cell: X by FONT_W(8), Y by FONT_H(16).
                crate::render::bitfont::blit_glyph(
                    &mut canvas, ch, sx + (start_col + i as u32) * 8, sy + last_row * 16, 8, 16, prompt_ink, Some(block),
                );
            }
        }
        raster_metrics = Some(rm);
    } else {
        // No usable text box — the picture owns the screen (SQ-0578), or there is
        // no story window at all. The story window's own plate still has to ship.
        v6::blit_story_gfx(&mut canvas, layout.story_gfx);
    }
    // Every layer has now drawn. Raster mode ships the WHOLE canvas as
    // one image, so any pixel still fully transparent would be resolved
    // by the compositor, not by us — kitty keeps the alpha and lets the
    // terminal decide, halfblocks flattens an untouched cell's
    // `Color::Reset` to WHITE. Paint those leftovers (the letterbox
    // margins around the frame art, and the story interior itself if a
    // full-bleed background tint inset `story_clear_native` to nothing)
    // with the same page, so the composite is self-contained and looks
    // identical on every protocol/terminal. Touches alpha==0 pixels
    // ONLY — art, status bands, glyphs and drop-caps are all opaque and
    // are left byte-for-byte alone. (SQ-0510)
    finish_v6_raster_canvas(canvas, page, raster_metrics)
}

/// Seal a v6 raster composite: resolve every still-transparent pixel to the story
/// page so the image is self-contained (SQ-0510). Shared by the normal tail of
/// [`build_v6_raster_canvas`] and its plate-owns-the-screen early return (SQ-0707).
fn finish_v6_raster_canvas(
    mut canvas: image::RgbaImage,
    page: image::Rgba<u8>,
    raster_metrics: Option<RasterMetrics>,
) -> (image::RgbaImage, Option<RasterMetrics>) {
    crate::render::v6_layout::flatten_onto_page(&mut canvas, page);
    (canvas, raster_metrics)
}

/// Flood every cell of `area` with an opaque game page colour (SQ-0532 wave-5).
///
/// Used by the two v6 pixel modes when the story window carries an EXPLICIT
/// background: the game's page is the whole screen's page, so the pane must show
/// it everywhere the scaled composite doesn't reach (letterbox margins) and
/// everywhere the composite is transparent (the chrome ring's clear pixels).
/// Drawn first, so the ring, the story viewport and the raster image all paint
/// over it.
fn fill_pane_page(area: Rect, page: image::Rgba<u8>, buf: &mut Buffer) {
    let style = ratatui::style::Style::new().bg(ratatui::style::Color::Rgb(page[0], page[1], page[2]));
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
}

/// A cheap change key for the whole v6 raster composite (SQ-0469). It folds
/// EVERY input the raster branch reads to build the native canvas — the v6 window
/// model, the transcript, the live input line, scroll/pager/caret state, the pane
/// size + font, and the themed colours — into one `u64`. When the key is
/// unchanged the entire rebuild + resize + encode is skipped, so idle and
/// keystroke frames cost only this hash (microseconds) instead of milliseconds.
///
/// A missed input here is a stale-frame bug, so the coverage is deliberately
/// generous (it hashes the built model's render fields rather than trusting a
/// hand-maintained zvm mutation counter — the model is observed, so no v6 paint
/// or erase can slip past). The inputs are audited in the SQ-0469 report.
pub fn v6_raster_gen(items: &[PositionedWindow], state: &AppState, area: Rect, picker: &ratatui_image::picker::Picker) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // Pane geometry + font size (drive the story cols/rows and the encode target).
    (area.width, area.height).hash(&mut h);
    let fs = picker.font_size();
    (fs.width, fs.height).hash(&mut h);
    // The v6 window model: each window's box geometry plus its render content —
    // graphics by version stamp (not pixels), text by its positioned runs and
    // colours. This observes the composited output, so any paint/erase/scroll or
    // colour change on the zvm side is captured without a bespoke counter.
    for pw in items {
        (pw.x, pw.y, pw.w, pw.h, pw.x_px, pw.y_px, pw.w_px, pw.h_px, pw.left_margin, pw.right_margin).hash(&mut h);
        match &pw.node {
            WinNode::Graphics(g) => {
                g.win.hash(&mut h);
                g.version.hash(&mut h);
            }
            WinNode::Grid(g) => {
                (g.bg, g.fg, g.cursor, g.cursor_active).hash(&mut h);
                for t in &g.px_texts {
                    (t.x, t.y, t.style, t.fg, t.bg).hash(&mut h);
                    t.text.hash(&mut h);
                }
            }
            WinNode::Buffer(b) => {
                (b.bg, b.fg, b.primary).hash(&mut h);
                // A SECONDARY prose window's lines are drawn into the composite
                // (SQ-0729), so a change to them must rebuild it — without this the
                // cached canvas outlives the text it was built from.
                if !b.primary {
                    b.lines.hash(&mut h);
                }
            }
            _ => {}
        }
    }
    // App-side inputs to build_main_text + the pager/caret.
    state.transcript_gen.hash(&mut h);
    state.transcript_images.len().hash(&mut h);
    state.input.value.hash(&mut h);
    state.effective_transcript_scroll().hash(&mut h);
    matches!(state.focus, crate::state::Focus::Game).hash(&mut h);
    state.pager.active.hash(&mut h);
    // The themed colours the raster resolves (default fg/bg + the [more] prompt);
    // a theme switch changes these even when the model is byte-identical.
    // A live `/set-game-colours` toggle changes the composite's page/ink
    // resolution without touching the model — it must invalidate the canvas.
    state.config.honor_game_colours.hash(&mut h);
    let tbg = state.colors.theme.get("transcript").style;
    style_fg_rgba(tbg, image::Rgba([220, 220, 220, 255])).0.hash(&mut h);
    style_bg_rgba(tbg, image::Rgba([0, 0, 0, 255])).0.hash(&mut h);
    let mp = state.colors.theme.get("more_prompt").style;
    style_fg_rgba(mp, image::Rgba([220, 220, 220, 255])).0.hash(&mut h);
    style_bg_rgba(mp, image::Rgba([0, 0, 0, 255])).0.hash(&mut h);
    h.finish()
}

/// Build the main-window text block for the pixel composite: the newest visible
/// wrapped transcript lines that fit the primary window's rows, plus the live
/// input line and caret column.
/// Build the v6 raster story text: wrap the transcript to the window width,
/// then place window-0 inline pictures (the `transcript_images` sidecar)
/// according to their `ImageAlign` (SQ-0470 follow-up). A `MarginLeft` image
/// floats — it occupies no text row, anchors at the next wrapped row, and
/// indents the `pic_height/8` rows beside it (Zork Zero's drop-cap idiom; the
/// indent comes from the game's own `set_margins` when it was captured). Every
/// other alignment (InlineUp/Down/Center, MarginRight — e.g. Shogun's ship
/// splash) is a full-width band: it reserves `pic_height/8` blank text rows so
/// prose stops above it and resumes below, never beside or over it. Keeps the
/// newest `rows-1` wrapped rows (one row is left for the input line).
pub fn build_main_text(state: &AppState, cols: u16, rows: u16) -> (crate::render::v6_layout::MainText, RasterMetrics) {
    // Non-square 8×16 v6 cell (SQ-0479). Picture pixels arriving here are already
    // in unit space (session scales v6 art ×2 before storing), so a float spans
    // height/FONT_H text rows and indents width/FONT_W columns.
    const FONT_W: u32 = 8;
    const FONT_H: u32 = 16;
    // A prose column narrower than this (cells) isn't worth floating a picture
    // beside — fall back to a full-width band.
    const MIN_TEXT_COLS: u16 = 8;
    struct AbsFloat {
        row: usize,
        rows: u16,
        /// Columns removed from the text width on the covered rows.
        reserve: u16,
        /// Column where covered rows' text begins.
        text_col: u16,
        /// Column where the picture blits.
        img_col: u16,
        img: std::sync::Arc<image::RgbaImage>,
    }
    // Columns reserved (subtracted from wrap width) by any float covering `row`.
    let reserve_at = |floats: &[AbsFloat], row: usize| -> u16 {
        floats
            .iter()
            .filter(|f| f.row <= row && row < f.row + f.rows as usize)
            .map(|f| f.reserve)
            .max()
            .unwrap_or(0)
    };
    // The §8.7.1 style bits the raster font can synthesize a face for (SQ-0540):
    // bold and italic. Reverse and fixed-pitch are meaningless in the prose
    // raster (no block to swap, one fixed-pitch face) and are dropped here.
    const EMPHASIS: u8 = 2 | 4;
    let mut wrapped: Vec<String> = Vec::new();
    // Per-char emphasis for wrapped rows that carry any, parallel to `wrapped`
    // and self-padding with empty (= all-roman) rows, so an unemphasised
    // transcript allocates nothing.
    let mut wrapped_styles: Vec<Vec<u8>> = Vec::new();
    fn set_row_styles(styles: &mut Vec<Vec<u8>>, row: usize, bits: Vec<u8>) {
        if bits.iter().all(|&b| b == 0) {
            return;
        }
        if styles.len() <= row {
            styles.resize(row + 1, Vec::new());
        }
        styles[row] = bits;
    }
    let mut floats: Vec<AbsFloat> = Vec::new();
    // Wrapped row each source line starts on, so the screen-clear anchor can be
    // mapped into the wrap — the raster twin of `wrap_lines_kinded_indexed`'s
    // `starts` (SQ-0640).
    let mut line_starts: Vec<usize> = Vec::with_capacity(state.transcript.len() + 1);
    for (i, line) in state.transcript.iter().enumerate() {
        line_starts.push(wrapped.len());
        if let Some(Some(img)) = state.transcript_images.get(i) {
            // ContentSplash entries exist only for frameless mode; the raster
            // path draws the graphics window canvas itself, so skip them here to
            // avoid double-rendering (SQ-0461). They still occupy no text row.
            if img.source == crate::inline_image::ImageSource::ContentSplash {
                continue;
            }
            let px = &img.pixels;
            // Rows the picture spans: ceil(h/FONT), so a picture whose height
            // isn't a cell multiple never has a full-width line drawn across
            // its bottom pixels. (Infocom's own countdown used floor and let
            // the overlap happen; with our whole-cell glyphs the ceil reads
            // far cleaner.)
            let img_rows = (px.height().div_ceil(FONT_H) as u16).max(1);
            let img_cols = (px.width().div_ceil(FONT_W) as u16).max(1);
            let band = |floats: &mut Vec<AbsFloat>, wrapped: &mut Vec<String>| {
                // A full-width band: reserve blank text rows so the wrap below
                // can't place prose beside or over the picture.
                floats.push(AbsFloat { row: wrapped.len(), rows: img_rows, reserve: 0, text_col: 0, img_col: 0, img: std::sync::Arc::clone(px) });
                for _ in 0..img_rows {
                    wrapped.push(String::new());
                }
            };
            match img.align {
                crate::inline_image::ImageAlign::MarginLeft => {
                    // A drop-cap floats at the LEFT: it occupies no text row of
                    // its own — the wrap below narrows the rows beside it, and the
                    // text is pushed right past the picture.
                    let indent_px = img.margin_px.unwrap_or(px.width() + FONT_W);
                    let reserve = indent_px.div_ceil(FONT_W) as u16;
                    floats.push(AbsFloat {
                        row: wrapped.len(),
                        rows: img_rows,
                        reserve,
                        text_col: reserve,
                        img_col: 0,
                        img: std::sync::Arc::clone(px),
                    });
                }
                crate::inline_image::ImageAlign::MarginRight => {
                    // A right-margin picture (Shogun's opening, ZMSD §15) floats at
                    // the RIGHT edge: text stays flush left and wraps in the
                    // narrowed column, then reclaims full width once the picture
                    // ends. Reserve the picture's own cell width plus a gutter; if
                    // that leaves no prose column, fall back to a full-width band.
                    let reserve = (img_cols + 1).min(cols);
                    if cols.saturating_sub(reserve) >= MIN_TEXT_COLS {
                        floats.push(AbsFloat {
                            row: wrapped.len(),
                            rows: img_rows,
                            reserve,
                            text_col: 0,
                            img_col: cols.saturating_sub(img_cols),
                            img: std::sync::Arc::clone(px),
                        });
                    } else {
                        band(&mut floats, &mut wrapped);
                    }
                }
                _ => band(&mut floats, &mut wrapped),
            }
            continue;
        }
        if line.is_empty() {
            wrapped.push(String::new());
            continue;
        }
        // Per-char emphasis for this logical line (SQ-0540), materialised only
        // when the line actually carries some — `transcript_runs` is parallel to
        // `transcript`, with char offsets into the UNWRAPPED line.
        let line_bits: Option<Vec<u8>> = state.transcript_runs.get(i).and_then(|runs| {
            runs.iter().any(|r| r.bits & EMPHASIS != 0).then(|| {
                let n = line.chars().count();
                let mut v = vec![0u8; n];
                for r in runs {
                    let end = r.end.min(n);
                    for b in v.iter_mut().take(end).skip(r.start.min(end)) {
                        *b = r.bits & EMPHASIS;
                    }
                }
                v
            })
        });
        // Slice `line_bits` for a wrapped row of `n` chars starting at source
        // char offset `from`. Wrapping only ever drops the single space at a
        // break, so each row is a contiguous run of the source line.
        let row_bits = |from: usize, n: usize| -> Vec<u8> {
            match &line_bits {
                Some(bits) => (0..n).map(|j| bits.get(from + j).copied().unwrap_or(0)).collect(),
                None => Vec::new(),
            }
        };
        // Word-wrap with per-row width: rows beside an active float are narrower.
        let mut cur = String::new();
        let mut cur_start = 0usize; // source char offset of `cur`'s first char
        let mut src = 0usize; // source char offset of `word`
        for word in line.split(' ') {
            let width = cols.saturating_sub(reserve_at(&floats, wrapped.len())).max(1) as usize;
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
                let n = cur.chars().count();
                wrapped.push(std::mem::take(&mut cur));
                set_row_styles(&mut wrapped_styles, wrapped.len() - 1, row_bits(cur_start, n));
            }
            if cur.is_empty() {
                cur_start = src;
            } else {
                cur.push(' ');
            }
            cur.push_str(word);
            src += word.chars().count() + 1; // +1 for the separating space
        }
        let n = cur.chars().count();
        wrapped.push(cur);
        set_row_styles(&mut wrapped_styles, wrapped.len() - 1, row_bits(cur_start, n));
    }
    // One row is reserved for the live input line, so the transcript body budget
    // is `rows - 1` — this is the raster viewport height the [more] pager and the
    // scroll keybindings measure against.
    let budget = rows.saturating_sub(1) as usize;
    let total = wrapped.len();
    let max_scroll = total.saturating_sub(budget);
    // Rows-from-bottom scroll offset (0 = newest at the bottom), clamped so it
    // never scrolls past the oldest row. Same scroll model as the terminal
    // transcript (`effective_transcript_scroll`), so the shared scroll keys and
    // the [more] pager (SQ-0404) drive the raster and terminal paths identically:
    // when the user scrolls back the visible slice shifts up in lockstep. (SQ-0455)
    let scroll = (state.effective_transcript_scroll() as usize).min(max_scroll);
    let mut end = total.saturating_sub(scroll);
    let mut start = end.saturating_sub(budget);
    // Top-anchor the post-clear screen, exactly as the cell path does
    // (`window_wrapped_rows`, SQ-0305/0640): at the bottom of the scrollback, a
    // game screen-clear pins its output to the TOP of the box with blanks below,
    // instead of bottom-sticking and dragging pre-clear history back into view.
    // Shogun's title needs it (SQ-0728): the SQ-0697 freeze retires nine banner
    // lines as paint and marks the clear, and window 0's new box is four rows —
    // bottom-sticking redrew the tail of the banner it had just frozen up top,
    // across the menu, instead of the one line the game printed into the new box.
    // Only while the post-clear content still fits; once it overflows, the box
    // scrolls normally.
    let anchor_row = (scroll == 0)
        .then(|| state.clear_anchor.and_then(|a| line_starts.get(a).copied()))
        .flatten()
        .map(|a| a.min(total));
    if let Some(a) = anchor_row.filter(|&a| total - a <= budget) {
        start = a;
        end = total;
    }
    let visible_len = end - start;
    let lines = wrapped[start..end].to_vec();
    // Emphasis travels with the visible slice (padded first: `wrapped_styles`
    // only reaches the last row that carried any).
    wrapped_styles.resize(total, Vec::new());
    let styles = wrapped_styles[start..end].to_vec();
    // Shift floats into the visible window; keep those still (partially) visible.
    let floats: Vec<crate::render::v6_layout::RasterFloat> = floats
        .into_iter()
        .filter_map(|f| {
            let rel = f.row as i64 - start as i64;
            (rel + f.rows as i64 > 0 && rel < visible_len as i64).then_some(crate::render::v6_layout::RasterFloat {
                row: rel as i32,
                rows: f.rows,
                reserve_cols: f.reserve,
                text_col: f.text_col,
                img_col: f.img_col,
                img: f.img,
            })
        })
        .collect();
    let input = state.input.value.clone();
    let cursor_col = input.chars().count().min(cols.saturating_sub(1) as usize) as u16;
    // Show the input line + caret whenever the view is at the bottom — scrolled-back
    // history must not be overwritten by the live line (matching the terminal
    // transcript's `effective_scroll == 0` guard).
    //
    // Deliberately NOT gated on host focus. It used to be, which meant the caret and
    // everything you had typed vanished the moment the keyboard went to the map —
    // opening a room panel, or reaching the inspector via select-room, hid your own
    // half-typed command with no indication it was still buffered. The Z-machine
    // transcript path has never had such a gate, so the two engines disagreed too.
    // Whether keystrokes currently reach the story is the focus HIGHLIGHT's job, not
    // the input line's.
    let awaiting = scroll == 0;
    let main = crate::render::v6_layout::MainText { lines, styles, input, cursor_col, awaiting, floats };
    let metrics = RasterMetrics {
        total_rows: total.min(u16::MAX as usize) as u16,
        viewport_rows: budget.min(u16::MAX as usize) as u16,
        max_scroll: max_scroll.min(u16::MAX as usize) as u16,
        first_visible_row: start.min(u16::MAX as usize) as u16,
    };
    (main, metrics)
}

/// Scroll/pager geometry the raster story text reports back so the [more] pager
/// (SQ-0404) and the transcript scroll keybindings engage on the raster path
/// exactly as they do on the terminal transcript. Rows are counted in the
/// raster's own 8-px text lines. (SQ-0455)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterMetrics {
    /// Total wrapped transcript rows this frame (the pager needs the true total).
    pub total_rows: u16,
    /// The transcript body viewport height in rows (`story_box_rows - 1`, the
    /// input line reserved).
    pub viewport_rows: u16,
    /// The largest meaningful scroll offset (`total_rows - viewport_rows`).
    pub max_scroll: u16,
    /// Absolute wrapped-row index drawn at the top of the visible slice (for the
    /// published `TranscriptGeom`).
    pub first_visible_row: u16,
}

/// How deep a chrome run must sit before the HYBRID path will treat a screen as a
/// painted MENU takeover (see the `has_menu` gate in the `Layered` arm). A run
/// this shallow is ordinary top-of-screen status chrome even when it happens to
/// land inside a story box that starts at row 0. (SQ-0478/SQ-0494)
const STATUS_BAND_ROWS: u16 = 4;

/// Render a v6 PAINTED text screen (menus, hints — SQ-0477/0478) as absolutely-
/// positioned terminal text. Each run is quantized to its native cell
/// (`col = (x-1)/8`, `row = (y-1)/16` — the non-square 8×16 v6 cell) and stamped at that pane-relative cell,
/// honoring reverse video — Shogun's boot-menu selection is a reverse-video run,
/// so this is what makes the selection caret visible. Menus are absolutely
/// positioned (NOT left/center/right anchor groups like the status band).
///
/// Only native rows inside `rows` are drawn, each placed at `area.y + row +
/// shift`. The frameless path calls this twice (SQ-0491): once for the runs
/// INSIDE the story box (`shift = 0` — Shogun's boot menu keeps its native rows
/// over the transcript) and once for the command band BELOW it (a negative or
/// positive `shift` that packs those rows against the pane's bottom edge, so
/// Journey's menu stays locked to the bottom at any pane height). A story-less
/// menu screen passes the whole range with no shift to stamp the entire pane.
/// Shared by the frameless and hybrid (no story window) paths so both present a
/// painted screen identically.
/// Resolve a v6 painted run's packed fg/bg (see [`crate::engine::PxText`]) plus
/// its reverse bit onto a `base` theme [`Style`], for the CELL render paths (the
/// frameless status band, the painted-screen overlay, and the hybrid story-strip
/// overlay). Mirrors the v1-5 / Glulx cell rule (`cell_style`): a run whose
/// channel carries an EXPLICIT game colour (see [`v6_layout::packed_explicit`])
/// replaces that channel; a Default or Standard-0/1 sentinel is inheritance, so
/// the theme keeps the channel. Gated on `honor` exactly like every other
/// engine's colour path — colours OFF ⇒ the theme `base` is returned untouched.
/// The reverse bit toggles REVERSED (the terminal performs the fg/bg swap), so
/// an explicit pair under reverse shows swapped and Shogun's Default/Default,
/// non-reversed runs collapse to exactly `base`. (SQ-0488)
fn v6_run_style(
    base: ratatui::style::Style,
    fg: u32,
    bg: u32,
    style_bits: u8,
    honor: bool,
    colors: &ColorScheme,
) -> ratatui::style::Style {
    let mut s = base;
    if honor {
        if crate::render::v6_layout::packed_explicit(fg) {
            s = s.fg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(fg), colors));
        }
        if crate::render::v6_layout::packed_explicit(bg) {
            s = s.bg(crate::render::resolve_zcolour(crate::state::unpack_zcolour(bg), colors));
        }
    }
    // ZMSD §8.7.1 style bits, as the model packs them (1 = reverse video,
    // 2 = bold, 4 = italic, 8 = fixed-pitch). Bold and italic used to be dropped
    // on every v6 cell path, so a game's emphasised menu text rendered roman.
    // They are ADDED when set (never removed): unlike REVERSED — which the
    // full-width flood rows below stamp into `base` and a non-reverse run must
    // clear — bold/italic only ever arrive from the run itself, so subtracting
    // them would fight the theme's own base style. Fixed-pitch (8) needs no
    // action in a monospaced terminal.
    if style_bits & 2 != 0 {
        s = s.add_modifier(ratatui::style::Modifier::BOLD);
    }
    if style_bits & 4 != 0 {
        s = s.add_modifier(ratatui::style::Modifier::ITALIC);
    }
    if style_bits & 1 != 0 {
        s.add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        s.remove_modifier(ratatui::style::Modifier::REVERSED)
    }
}

/// A status STRIP the game paints OVER the top of its own story window, rather than
/// above it (SQ-0582). advent.z6 leaves window 0 covering the whole 640×380 screen and
/// hangs window 1 — full width, one row tall, pinned at the top — over its first row.
/// Every other v6 game here reserves the band by placing the story window BELOW it
/// (Zork0 y=79, Shogun y=33, Arthur y=209), so the chrome ring picks the band up for
/// free; with an overlay there is no ring at all, and the bar's runs land inside the
/// story box to be stamped glyph-by-glyph over the transcript — a ribbon with holes
/// between the fields, and the transcript scrolling underneath it.
///
/// Returns the overlaying window, whose rows the caller reserves at the top of the
/// story viewport so the ordinary Text-strip path draws it as a solid bar.
fn overlaid_status_strip<'a>(
    chrome: &[&'a PositionedWindow],
    story: &PositionedWindow,
    native_w: u16,
) -> Option<&'a PositionedWindow> {
    let threshold = native_w as u32 * 9 / 10;
    chrome.iter().copied().find(|pw| {
        let WinNode::Grid(g) = &pw.node else { return false };
        // Text INSIDE the window's own rect. v6 text is paint and outlives the
        // window's geometry (ZMSD §8.8.4), so a window shrunk to a sliver while its
        // message box sits 50px lower — advent's own boot popup — is not a bar, and
        // reserving rows for it would inset the story out from under the popup.
        strip_rows(pw, g).is_some()
            && pw.w_px as u32 >= threshold
            && pw.h_px > 0
            && pw.h_px <= V6_STATUS_STRIP_MAX_H_PX
            && pw.y_px <= story.y_px
            && pw.y_px.saturating_add(pw.h_px) > story.y_px
    })
}

/// The native rows a status strip's own text occupies: the last row carrying a
/// non-blank run that lies within the window's rect, or `None` when it paints none
/// there. (SQ-0582/SQ-0584)
fn strip_rows(pw: &PositionedWindow, g: &crate::engine::GridWindow) -> Option<u16> {
    let first = pw.y_px / 16;
    let last = pw.y_px.saturating_add(pw.h_px).div_ceil(16).max(first + 1);
    g.px_texts
        .iter()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| (t.y.max(1) - 1) / 16)
        .filter(|row| *row >= first && *row < last)
        .max()
}

/// Tallest v6 grid window that counts as a status STRIP for [`full_width_flood_rows`]:
/// two text rows. Matches `zvm::location`'s band rule, which mines the same shape for
/// the room name — a bar is one or two rows, anything taller is a panel or overlay.
const V6_STATUS_STRIP_MAX_H_PX: u16 = 32;

/// SQ-0515/SQ-0582: which native rows of a painted v6 screen should flood edge-to-edge
/// with a bar (see [`draw_painted_screen`]). A row qualifies only when each of its
/// non-blank runs belongs to a grid window spanning at least ~90% of the native screen
/// width — a full-width title/status bar (Zork0's " InvisiClues (tm)" header sits in a
/// w_px=640/640 window) rather than a narrow selection block (Shogun's boot-menu window
/// is w_px=169/640, and Zork0's own selected-topic highlight is in the w_px=468/640
/// topic window — both stay text-width) — AND one of:
///
///   - every run on it is reverse-video (the game asked for a bar), or
///   - the owning window is a STRIP at most [`V6_STATUS_STRIP_MAX_H_PX`] tall AND the
///     run lies INSIDE that window's own rect. Not every game styles its status line:
///     advent.z6 paints "At End Of Road … Score: 36 … Moves: 1" into a full-width,
///     one-row window with no reverse bit and no colours (SQ-0582), so the reverse
///     rule never fired and the theme's bar background reached only the cells under
///     the glyphs — a ribbon with holes between the fields. A window that shape IS the
///     status bar, whatever style its text carries.
///
///     The containment half is not pedantry: v6 text is PAINT, and a run stays where
///     it was put even when its window is later resized to nothing (ZMSD §8.8.4 — a
///     window's size "does not change the current display"). advent's own boot popup
///     is exactly that — window 1 shrunk to 640x1 while it paints a message box 50px
///     down the screen — so height alone called every popup row a status bar and
///     flooded the story text behind it edge to edge.
///
/// Returns native_row → flood [`Style`], with colours resolved first-explicit-wins
/// across the row and the reverse bit set only for the reverse case, via
/// [`v6_run_style`].
fn full_width_flood_rows(
    chrome: &[&PositionedWindow],
    native_w: u16,
    base: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
) -> std::collections::HashMap<u16, ratatui::style::Style> {
    use crate::render::v6_layout::packed_explicit;
    let threshold = native_w as u32 * 9 / 10;
    // Group every non-blank run by native row, carrying its owning window's size.
    let mut per_row: std::collections::HashMap<u16, Vec<(&crate::engine::PxText, u16, u16, bool)>> = Default::default();
    for pw in chrome {
        if let WinNode::Grid(g) = &pw.node {
            for t in &g.px_texts {
                if t.text.trim().is_empty() {
                    continue;
                }
                let row = (t.y.max(1) - 1) / 16;
                // Does this run sit within the rows its own window covers?
                let win_first = pw.y_px / 16;
                let win_last = pw.y_px.saturating_add(pw.h_px).div_ceil(16).max(win_first + 1);
                let inside = row >= win_first && row < win_last;
                per_row.entry(row).or_default().push((t, pw.w_px, pw.h_px, inside));
            }
        }
    }
    let mut out = std::collections::HashMap::new();
    for (row, row_runs) in per_row {
        if !row_runs.iter().all(|(_, w, _, _)| *w as u32 >= threshold) {
            continue;
        }
        let all_reverse = row_runs.iter().all(|(t, _, _, _)| t.style & 1 != 0);
        let all_strip =
            row_runs.iter().all(|(_, _, h, inside)| *inside && *h > 0 && *h <= V6_STATUS_STRIP_MAX_H_PX);
        if !all_reverse && !all_strip {
            continue;
        }
        let fg = row_runs.iter().map(|(t, _, _, _)| t.fg).find(|&p| packed_explicit(p)).unwrap_or(0);
        let bg = row_runs.iter().map(|(t, _, _, _)| t.bg).find(|&p| packed_explicit(p)).unwrap_or(0);
        // Reverse only where the game asked for it: a plain strip floods with the
        // theme's own bar style, exactly as the anchored band does.
        let style_bits = u8::from(all_reverse);
        out.insert(row, v6_run_style(base, fg, bg, style_bits, honor, colors));
    }
    out
}

/// Paint the background fields left by `erase_window` (SQ-0584).
///
/// ZMSD §8.8.5.3: erasing a window fills its rect with that window's background, and
/// on a real interpreter — where every v6 window is a clipping region over ONE screen
/// bitmap — that fill is opaque paint covering whatever was under it. A window carries
/// `fill` only while it is still the newest paint on its own rect (see
/// `GridWindow::fill`), so an ordinary turn, whose prose is newer, paints nothing here.
///
/// Drawn in the order the game erased them, over the whole pane rather than any one
/// draw call's row window: advent.z6's `help` erases the full screen and then its
/// 160px menu window, and the real screen that leaves is a menu panel on blank
/// background — not a panel with the transcript resuming under it.
/// Draw the v6 SECONDARY prose windows: flowing-text windows that are not the one
/// the player types into (SQ-0585). A v6 game may run several at once — advent.z6's
/// `style` opens one across the top of the screen and keeps playing in another below
/// — and the engine keeps each one's text in its own window rather than splicing them
/// into the transcript, so each draws in its own rect here.
///
/// Live screen state: what the window currently holds, no scrollback. Drawn after the
/// erase fills (a window is erased, then printed into) and before the chrome runs, so
/// a status bar painted over the same rows still lands on top.
fn draw_secondary_buffers(
    chrome: &[&PositionedWindow],
    area: Rect,
    buf: &mut Buffer,
    state: &AppState,
    to_cells: &dyn Fn(&PositionedWindow) -> Rect,
) {
    for pw in chrome {
        let WinNode::Buffer(b) = &pw.node else { continue };
        if b.primary || b.lines.is_empty() {
            continue;
        }
        let r = to_cells(pw);
        let clipped = Rect::new(
            r.x.max(area.x),
            r.y.max(area.y),
            r.width.min(area.right().saturating_sub(r.x.max(area.x))),
            r.height.min(area.bottom().saturating_sub(r.y.max(area.y))),
        );
        if clipped.width == 0 || clipped.height == 0 {
            continue;
        }
        render_inline_buffer(b, state, clipped, buf);
    }
}

fn draw_erase_fills(
    chrome: &[&PositionedWindow],
    area: Rect,
    buf: &mut Buffer,
    base: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
    to_cells: &dyn Fn(&PositionedWindow) -> Rect,
) {
    let mut fills: Vec<(&PositionedWindow, crate::engine::ErasedFill)> = chrome
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => g.fill.map(|f| (*pw, f)),
            _ => None,
        })
        .collect();
    fills.sort_by_key(|(_, f)| f.seq);
    for (pw, f) in fills {
        let style = v6_run_style(base, 0, f.bg, 0, honor, colors);
        let r = to_cells(pw);
        for y in r.y.max(area.y)..r.bottom().min(area.bottom()) {
            for x in r.x.max(area.x)..r.right().min(area.right()) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    }
}

fn draw_painted_screen(
    runs: &[&crate::engine::PxText],
    rows: std::ops::Range<u16>,
    shift: i32,
    area: Rect,
    buf: &mut Buffer,
    base: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
    chrome: &[&PositionedWindow],
    native_w: u16,
) {
    // A native row's terminal row, or `None` when it is outside this call's row
    // range or the shift pushes it off the pane.
    let place = |row: u16| -> Option<u16> {
        if !rows.contains(&row) {
            return None;
        }
        let y = area.y as i32 + row as i32 + shift;
        (y >= area.y as i32 && y < area.bottom() as i32).then_some(y as u16)
    };
    // SQ-0515: a native row whose non-blank runs are ALL reverse-video and all
    // belong to a (near-)full-native-width grid window is a title/status bar —
    // flood the whole terminal row edge to edge with the reversed style before
    // stamping the glyphs, so Zork0's " InvisiClues (tm)" header reads as a solid
    // reverse bar rather than reverse across only its own glyphs. A narrow window's
    // reverse row (Shogun's boot-menu selection) is untouched — it stays a
    // text-width highlight block below.
    let flood = full_width_flood_rows(chrome, native_w, base, honor, colors);
    for (&row, &style) in &flood {
        let Some(y) = place(row) else { continue };
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
    }
    for t in runs {
        // Every run stamps, whitespace included — painter semantics. A reversed
        // space fills its cell of the selection bar (SQ-0484), and a NORMAL
        // space must equally repaint over an earlier reversed one: when the
        // menu selection moves, the game repaints the old row's gaps as plain
        // spaces, and skipping those left the stale reversed cells behind
        // (SQ-0490).
        // 8×16 v6 cell (SQ-0479): quantize Y by FONT_H(16), X by FONT_W(8).
        let row = (t.y.max(1) - 1) / 16;
        let Some(y) = place(row) else { continue };
        let col = (t.x.max(1) - 1) / 8;
        if area.x + col >= area.right() {
            continue;
        }
        // Cell styles PATCH — a repaint must explicitly clear the reverse bit,
        // or a cell once reversed stays reversed after the game repaints it
        // plain (SQ-0490). Explicit game colours on the run replace the theme
        // base per channel; inherited/Default channels keep it (SQ-0488).
        let style = v6_run_style(base, t.fg, t.bg, t.style, honor, colors);
        let max_w = (area.right() - (area.x + col)) as usize;
        // Untrusted game text (SQ-0639): a control char would shift the rest of
        // the run a column left, and these runs are pixel-placed.
        let text = crate::render::blank_control_chars(&t.text);
        buf.set_stringn(area.x + col, y, text.as_ref(), max_w, style);
    }
}

/// One horizontal strip of a full-width hybrid chrome band (SQ-0500): either
/// `Art` (opaque frame graphics behind it — keep the scaled pixel ring) or `Text`
/// (no graphics behind, only status/menu runs — paint as terminal cells). The
/// runs carried by a `Text` strip are the chrome grid runs that map into it.
enum ChromeStrip<'a> {
    Art(Rect),
    Text(Rect, Vec<&'a crate::engine::PxText>),
}

/// SQ-0505 dynamic hybrid layout: how the vertical letterbox slack below the
/// story window is reclaimed.
///   `Letterbox` — keep today's centred frame (Zork0's enclosed full frame, or a
///                 pane with no slack to reclaim).
///   `Extend`    — top-anchor the ring and grow the story viewport to the pane
///                 bottom (Arthur: header art + side borders, open below).
///   `Menu`      — top-anchor the story/chrome and bottom-anchor a text command
///                 strip; the story fills between (Journey's command menu).
///   `Frame`     — top-anchor the story and grow it to the pane bottom, but keep
///                 the ENCLOSING side art by stretching the flank bands vertically
///                 to span the reclaimed space (SQ-0511: Zork0/Shogun, whose frame
///                 reaches the native bottom and is flanked by full-height side art).
enum BottomPlan {
    Letterbox,
    Extend,
    Menu,
    Frame,
}

/// SQ-0570: is this frame a full-screen PICTURE takeover — a picture painted
/// across the whole screen with the story window grown over it?
///
/// Zork Zero's `map` command is the case. It is the exact inverse of the title
/// splash: the splash calls `split_window(400)` so window 1 becomes the screen and
/// window 0 COLLAPSES to zero height, leaving no story viewport to carve (SQ-0497),
/// whereas the map GROWS window 0 to the full screen `(0,0) 640×400` and paints the
/// map into the full-screen graphics window beneath it. Hybrid mode then made the
/// story viewport the entire pane, which leaves `chrome_bands` empty — so the map
/// was never uploaded at all and the transcript painted over the whole screen. (It
/// reads as a sudden drop into frameless mode: no frame, no picture, just text.)
///
/// Such a frame has no ring to draw, so there is nothing for hybrid to do: the
/// caller falls through to the RASTER composite, which draws the picture and
/// rasterizes the story text over it in one canvas, and already renders this screen
/// correctly. Detection is deliberately narrow — the story window must cover the
/// whole screen (within one native text row per edge) AND opaque graphics must sit
/// behind it, either FILLING it or FRAMING it (below). An ordinary gameplay screen
/// keeps window 0 inset inside its frame, so it can never qualify.
///
/// The opacity test samples a coarse grid rather than every pixel: it runs on every
/// hybrid frame, and a fully painted picture versus a frame-only (or empty) canvas
/// is not a close call.
///
/// SQ-0729: filling the screen was too strong a test. fmvpoker paints a 640×400
/// poker table into full-screen window 0 and prints its whole title inside it, and
/// that table is a FRAME — 17% of its pixels opaque, the middle a hole — so the
/// grid below misses it at every point that matters and hybrid kept its (empty)
/// ring. The game drew not one picture on screen. What actually decides this is
/// the story window covering the screen: that alone leaves `chrome_bands` with
/// nothing to carve, so NO art behind it can be uploaded whatever its shape. So a
/// second arm asks whether the art ENCLOSES the screen instead — painted pixels
/// within a native text row of all four edges, which is "the painted bounding box
/// spans the screen" without scanning the interior. Measured across the v6 corpus,
/// this moves fmvpoker alone: Zork Zero and Shogun keep window 0 inset, advent and
/// scopa paint nothing behind it, Arthur's intro plate and Journey's title already
/// fill the screen, and mysterious01's plate (a 512×192 band across the lower half)
/// reaches neither the top edge nor the right one.
///
/// SQ-0739: a third way to have no ring to draw. The ring's bands are cropped from
/// the CHROME canvas, and the story window's own plate is deliberately not in it —
/// it belongs to the story and is blitted inside the story viewport instead. That
/// holds only while the plate is inside the window it belongs to. When it is not,
/// the escaping art is in neither place: no band carries it and no viewport shows
/// it. See [`story_plate_escapes_story_window`].
fn picture_takeover(
    story: &crate::engine::PositionedWindow,
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    (story_covers_screen(story, native)
        && (art_fills_screen(chrome, story_gfx, native) || art_encloses_screen(chrome, story_gfx, native)))
        || story_plate_escapes_story_window(story, story_gfx)
}

/// SQ-0739: does the STORY window's own plate paint outside the story window's box?
///
/// Hybrid splits the screen in two: the chrome ring carries every pixel outside the
/// story viewport, and the story viewport is terminal cells with the story window's
/// own plate blitted into it as a float. `classify_windows` sets that plate aside as
/// `story_gfx` precisely so the ring does NOT carry it — which is right exactly as
/// long as the plate lives inside the window it is the plate OF.
///
/// fmvpoker breaks that assumption without redrawing anything. Choosing "Change
/// Current Bet" makes its 594x156 bottom panel the window the game reads input
/// through, so the panel becomes the primary Buffer and window 0 — still holding
/// the 640x400 poker table drawn into it — stops being the story window. The table
/// did not move; the story window did. The plate then paints across the whole
/// screen while the story viewport is one panel at the bottom, so the frame belongs
/// to neither half: `build_chrome_canvas` never sees it (it is not chrome) and the
/// viewport is far too small to show it. The ring came up with a canvas of ZERO
/// opaque pixels and the player's frame vanished for the duration of the bet — the
/// same "sudden drop into frameless mode" this function was written for.
///
/// Cheap first: a plate the story window's box CONTAINS cannot escape it, which is
/// every corpus frame that has a plate at all (Arthur's intro, Journey's title,
/// mysterious01 and fmvpoker's own steady state all publish the plate at exactly the
/// story window's box). Only when the boxes disagree is the alpha sampled, and then
/// on a coarse grid — a plate that reaches outside reaches by whole bands, never by
/// a stray pixel.
pub fn story_plate_escapes_story_window(
    story: &crate::engine::PositionedWindow,
    story_gfx: Option<&crate::engine::PositionedWindow>,
) -> bool {
    let Some(pw) = story_gfx else { return false };
    let crate::engine::WinNode::Graphics(gw) = &pw.node else { return false };
    let (px0, py0) = (pw.x_px as u32, pw.y_px as u32);
    let (px1, py1) = (px0 + gw.canvas.width(), py0 + gw.canvas.height());
    let (sx0, sy0) = (story.x_px as u32, story.y_px as u32);
    let (sx1, sy1) = (sx0 + story.w_px as u32, sy0 + story.h_px as u32);
    if px0 >= sx0 && py0 >= sy0 && px1 <= sx1 && py1 <= sy1 {
        return false;
    }
    const STEP: usize = 4;
    (py0..py1).step_by(STEP).any(|y| {
        (px0..px1).step_by(STEP).any(|x| {
            (x < sx0 || x >= sx1 || y < sy0 || y >= sy1) && gw.canvas.get_pixel(x - px0, y - py0)[3] >= 128
        })
    })
}

/// One native text row of slack per edge, so a game that leaves a hairline border
/// still counts as covering the screen.
const SCREEN_SLOP: u32 = 16;

/// Is a pixel painted by any of these graphics windows?
fn art_painted_probe<'a>(
    chrome: &'a [&'a crate::engine::PositionedWindow],
    story_gfx: Option<&'a crate::engine::PositionedWindow>,
) -> impl Fn(u32, u32) -> bool + 'a {
    let painted_at = |x: u32, y: u32, pw: &crate::engine::PositionedWindow| {
        let crate::engine::WinNode::Graphics(gw) = &pw.node else { return false };
        let (wx, wy) = (pw.x_px as u32, pw.y_px as u32);
        let img = &gw.canvas;
        x >= wx
            && y >= wy
            && x - wx < img.width()
            && y - wy < img.height()
            && img.get_pixel(x - wx, y - wy)[3] >= 128
    };
    move |x: u32, y: u32| {
        chrome.iter().any(|pw| painted_at(x, y, pw)) || story_gfx.is_some_and(|pw| painted_at(x, y, pw))
    }
}

/// Does the artwork FILL the screen — a solid plate the game narrates over?
///
/// Sampled on a coarse 8×8 grid rather than every pixel: this runs on every hybrid
/// frame, and a fully painted picture versus a frame-only (or empty) canvas is not
/// a close call. The STORY window's own plate counts, not just chrome: Arthur's
/// intro erases every window, centres a 584×392 illustration inside full-screen
/// window 0 and narrates over it (SQ-0695), and there is no chrome ring at all on
/// those screens — scanning chrome alone found nothing painted and hybrid opened a
/// pane-wide transcript viewport over art it then never uploaded.
fn art_fills_screen(
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    const N: u32 = 8;
    let painted = art_painted_probe(chrome, story_gfx);
    (0..N).all(|iy| {
        let y = native.1 as u32 * (2 * iy + 1) / (2 * N);
        (0..N).all(|ix| {
            let x = native.0 as u32 * (2 * ix + 1) / (2 * N);
            painted(x, y)
        })
    })
}

/// Does this story window cover the whole screen (within [`SCREEN_SLOP`] per edge)?
/// Such a window leaves hybrid's `chrome_bands` with nothing to carve.
fn story_covers_screen(story: &crate::engine::PositionedWindow, native: (u16, u16)) -> bool {
    (story.x_px as u32) <= SCREEN_SLOP
        && (story.y_px as u32) <= SCREEN_SLOP
        && story.x_px as u32 + story.w_px as u32 + SCREEN_SLOP >= native.0 as u32
        && story.y_px as u32 + story.h_px as u32 + SCREEN_SLOP >= native.1 as u32
}

/// Does the artwork ENCLOSE the screen (SQ-0729) — painted pixels within one native
/// text row of every edge? Probed edge strip by edge strip, so a hollow FRAME
/// answers on its border instead of failing on its hole, which is what makes this
/// different from "the art fills the screen".
///
/// Corpus-measured when it was written, and the measurement is what makes it safe
/// to reuse: it fires for exactly one title, fmvpoker. Zork Zero and Shogun keep
/// window 0 inset inside their frames, advent and scopa paint nothing behind it,
/// Arthur's intro plate and Journey's title are solid and answer the FILL test
/// instead, and mysterious01's plate is a 512×192 band across the lower half that
/// reaches neither the top edge nor the right one.
fn art_encloses_screen(
    chrome: &[&crate::engine::PositionedWindow],
    story_gfx: Option<&crate::engine::PositionedWindow>,
    native: (u16, u16),
) -> bool {
    let painted = art_painted_probe(chrome, story_gfx);
    let (w, h) = (native.0 as u32, native.1 as u32);
    let any_painted = |xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
        ys.step_by(2).any(|y| xs.clone().step_by(2).any(|x| painted(x, y)))
    };
    any_painted(0..w, 0..SCREEN_SLOP)
        && any_painted(0..w, h.saturating_sub(SCREEN_SLOP)..h)
        && any_painted(0..SCREEN_SLOP, 0..h)
        && any_painted(w.saturating_sub(SCREEN_SLOP)..w, 0..h)
}

/// SQ-0729: is this story window a CANVAS rather than a page — a window the game
/// has drawn a frame AROUND and then positions text INSIDE, rather than a
/// transcript it narrates on?
///
/// The discriminator is deliberately not "what does this RUN mean". A `set_cursor`
/// before a run is genuinely ambiguous: Arthur positions every room headline in
/// window 0 (one character at a time, only the first carrying the declaration),
/// Shogun and Journey centre each header line the same way, and mysterious01
/// re-homes before its prompt — all of them meaning "resume the transcript here",
/// while fmvpoker's HOLD means "paint this under that card". Nothing in the signal
/// separates them, which is what a measured attempt at that rule established.
///
/// So the question asked here is what kind of SURFACE the window is. Arthur's
/// window 0 is a transcript that happens to have plates drawn on it; fmvpoker's is
/// a picture frame that happens to have text positioned in it — its own art
/// encloses it on all four sides and it covers the whole screen. That is the same
/// test [`picture_takeover`]'s enclosure arm asks, reused rather than restated so
/// the two cannot drift apart, and it fires for fmvpoker alone.
///
/// It also extends a rule this codebase already made: SQ-0711/SQ-0716 ruled that a
/// window the game has drawn into is a canvas and keeps the ground it drew on.
/// This says the same of the text on that ground.
///
/// ENCLOSING and not FILLING is the whole discriminator, and both halves are
/// load-bearing. A solid full-screen plate reaches all four edges too, so Journey's
/// title read as a canvas until the fill test excluded it — and a plate is a
/// picture a game NARRATES OVER (Arthur's illustrated screens, Journey's title),
/// while a frame with a hole in the middle is a picture a game POSITIONS TEXT
/// INSIDE. [`picture_takeover`] takes either, because for its purposes — hybrid has
/// no ring to draw — the two are the same.
pub fn story_window_is_a_canvas(
    layout: &crate::render::v6_layout::V6Layout<'_>,
    native: (u16, u16),
) -> bool {
    layout.story.is_some_and(|s| story_covers_screen(s, native))
        && !art_fills_screen(&layout.chrome, layout.story_gfx, native)
        && art_encloses_screen(&layout.chrome, layout.story_gfx, native)
}

/// Classify what sits below the story window natively, to pick the [`BottomPlan`]
/// (SQ-0505). `slack` is the vertical letterbox margin in device pixels.
///
/// Keeps the centred letterbox when there is no slack to reclaim, when the story
/// already reaches the native screen bottom (its frame encloses it — Zork0's story
/// bottom is 398 of 400), or when a real ART band spans the story columns below it
/// (rule 4). Otherwise the below-story region is text-only (→ `Menu`) or empty
/// (→ `Extend`). The art test is restricted to the STORY COLUMNS so full-height
/// side borders (which flank, not floor, the story) never read as a bottom band.
fn hybrid_bottom_plan(
    story: &crate::engine::PositionedWindow,
    gfx: &image::RgbaImage,
    chrome_runs: &[&crate::engine::PxText],
    native: (u16, u16),
    slack: u32,
) -> BottomPlan {
    if slack == 0 {
        return BottomPlan::Letterbox;
    }
    let story_bottom = story.y_px as u32 + story.h_px as u32;
    // Story fills to (within one native row of) the screen bottom → enclosed frame.
    // SQ-0511: when full-height side ART flanks the story on BOTH sides, reclaim the
    // slack via the `Frame` plan (top-anchor the story to the pane bottom, stretch
    // the flanks to keep the enclosing columns). Zork0 (story bottom 398/400) and
    // Shogun (400/400) both qualify. With no side art there is nothing to stretch, so
    // keep the centred letterbox.
    if native.1 as u32 <= story_bottom + 16 {
        let sy0 = story.y_px as u32;
        let sy1 = story_bottom.min(gfx.height());
        let sx0 = story.x_px as u32;
        let sx1 = (story.x_px as u32 + story.w_px as u32).min(gfx.width());
        let flank_opaque = |xa: u32, xb: u32| -> bool {
            xa < xb && (sy0..sy1).any(|y| (xa..xb).any(|x| gfx.get_pixel(x, y)[3] >= 128))
        };
        let left_art = flank_opaque(0, sx0.min(gfx.width()));
        let right_art = flank_opaque(sx1, gfx.width());
        if left_art && right_art {
            return BottomPlan::Frame;
        }
        // SQ-0571: with no enclosing side art there is nothing to stretch, so
        // top-anchor (`Extend`) rather than CENTRE the frame. Centring made the
        // whole screen's position depend on the story window's height, and Arthur
        // changes that height mid-game: `map` grows win0 from 128 to 192 native px
        // (bottom 400), and its F6 text screen opens win0 at 640×384 (bottom 400),
        // both of which flipped the plan Extend → Letterbox. The centred offset then
        // dropped everything — header art, the map drawn into it, or a bare text
        // page — half the letterbox slack down the pane, and dismissing the screen
        // shrank the window and jumped it all back to the top. `Extend` simply lets
        // the story fill to the pane bottom, exactly as it does at the smaller
        // window height, so nothing moves. Zork0/Shogun are unaffected: their
        // full-height side art takes the `Frame` arm above.
        return BottomPlan::Extend;
    }
    let sx0 = story.x_px as u32;
    let sx1 = (story.x_px as u32 + story.w_px as u32).min(gfx.width());
    let colw = sx1.saturating_sub(sx0);
    // A genuine bottom ART band covers most of the story columns below the window.
    let art_band = colw > 0
        && (story_bottom..native.1 as u32).any(|y| {
            let cnt = (sx0..sx1).filter(|&x| gfx.get_pixel(x, y)[3] >= 128).count() as u32;
            cnt * 2 >= colw
        });
    if art_band {
        return BottomPlan::Letterbox;
    }
    let text_below = chrome_runs
        .iter()
        .any(|t| !t.text.trim().is_empty() && (t.y.max(1) as u32 - 1) >= story_bottom);
    if text_below {
        BottomPlan::Menu
    } else {
        BottomPlan::Extend
    }
}

/// SQ-0511: the native crop a stretched side flank `band` samples. Its columns are
/// the flank's native columns (band device x-range inverted through the uniform
/// `scale`, so the horizontal factor stays `s`); its rows run from the flank art's
/// top (the band device top inverted through the same scale — continuous with the
/// top band above) down to `native_bottom` (the flank art's bottom for a Frame, the
/// story bottom for a Menu). Returns `None` when the crop is empty. `native` bounds
/// the crop to the canvas.
fn flank_crop(
    band: Rect,
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    native_bottom: u32,
    native: (u16, u16),
) -> Option<(u32, u32, u32, u32)> {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    let s = if scale.s <= 0.0 { 1.0 } else { scale.s };
    let inv_x = |cell: u16| (((cell.saturating_sub(pane.x)) as f32 * cw - scale.off_x as f32) / s).round().max(0.0) as u32;
    let inv_y = |cell: u16| (((cell.saturating_sub(pane.y)) as f32 * ch - scale.off_y as f32) / s).round().max(0.0) as u32;
    let nx0 = inv_x(band.x).min(native.0 as u32);
    let nx1 = inv_x(band.right()).min(native.0 as u32);
    let ny0 = inv_y(band.y).min(native.1 as u32);
    let ny1 = native_bottom.min(native.1 as u32);
    if nx1 <= nx0 || ny1 <= ny0 {
        return None;
    }
    Some((nx0, ny0, nx1 - nx0, ny1 - ny0))
}

/// SQ-0511 fix (Journey Menu plan): the divider/border-column extension for one side
/// flank. The flank picture is drawn at the UNIFORM scale (aspect preserved), so a gap
/// opens between the flank art's uniform-scaled bottom (the story's native bottom) and
/// the bottom-anchored menu. This returns a NARROW band spanning that gap over the
/// flank's full-height border column — the reversed-run divider abutting the story on
/// the LEFT flank, the matching border on the RIGHT — plus a 1-native-pixel crop of
/// those columns to replicate down the gap. The column is uniform, so the vertical
/// replicate is invisible; the rest of the gap is left undrawn (transparent → theme
/// backdrop, matching the flank's own never-painted background beside the divider).
/// Returns `None` when the flank has no border column abutting the story or the gap is
/// empty. `menu_top_row` is the bottom-anchored menu strip's top cell (viewport bottom).
/// What [`menu_flank_panel`] resolves for a side flank: the panel background to
/// flood the column with, the destination rect for the vertically centred art,
/// and the native `(x, y, w, h)` crop of the canvas to draw into it.
type FlankPanel = (image::Rgba<u8>, Rect, (u32, u32, u32, u32));

/// SQ-0547: treat a Menu-plan side flank as a PANEL rather than a top-anchored
/// strip of art over bare backdrop.
///
/// Journey's left column holds an illustration far shorter than the column is at
/// a tall pane, so the reclaimed space below it showed the theme backdrop and the
/// column stopped reading as part of the game. Returns
/// `(panel background, destination rect for the art, native crop)`:
///
///   * the background is the game's OWN panel colour, sampled from the outer edge
///     of the flank art — the colour Journey paints around its picture (rgb 34,34,34)
///     — so the filled column matches the art instead of the theme or the letterbox;
///   * the destination rect keeps the band's horizontal placement exactly and
///     centres the art VERTICALLY in the column, at the uniform scale (the art's
///     own aspect ratio is preserved — SQ-0511's fix must not regress);
///   * the crop is the art's opaque row span across the flank's native columns.
///
/// `None` when the flank carries no art at all: there is then nothing to centre
/// and no colour to sample, so the caller keeps today's behaviour.
///
/// Measured against the GRAPHICS-only canvas (`gfx`), never the full chrome
/// canvas: the latter has the game's text rasterized into it, so its first opaque
/// pixel in this column is a light text band and both the row span and the
/// sampled panel colour would come out wrong. Same canvas the strip decomposition
/// consults to answer "is there art behind this strip?".
fn menu_flank_panel(
    band: Rect,
    viewport: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
    gfx: &image::RgbaImage,
    divider: Option<Rect>,
) -> Option<FlankPanel> {
    if band.width == 0 || band.height == 0 {
        return None;
    }
    // The flank's native column range: left of the story box, or right of it.
    let story_x0 = story.x_px as u32;
    let story_x1 = (story.x_px as u32 + story.w_px as u32).min(native.0 as u32);
    let (nx0, nx1) = if band.x < viewport.x {
        (0, story_x0.min(gfx.width()))
    } else {
        (story_x1.min(gfx.width()), (native.0 as u32).min(gfx.width()))
    };
    if nx1 <= nx0 {
        return None;
    }
    // The art's opaque BOUNDING BOX in that column, plus the first opaque pixel on
    // its top row — the art's outer edge, i.e. the panel colour the game painted.
    //
    // The box must be tight on BOTH axes. Journey's picture occupies native x 5..226
    // of a 240-wide column, so cropping the whole column would drag its transparent
    // side margins into the drawn image, and those render as dark cells ON TOP of the
    // panel fill — a strip of "missing background" down the right of the picture.
    let mut top: Option<(u32, image::Rgba<u8>)> = None;
    let (mut ax0, mut ax1, mut ay1) = (u32::MAX, 0u32, 0u32);
    for y in 0..gfx.height() {
        let mut row_first: Option<u32> = None;
        for x in nx0..nx1 {
            if gfx.get_pixel(x, y)[3] >= 128 {
                if row_first.is_none() {
                    row_first = Some(x);
                }
                ax0 = ax0.min(x);
                ax1 = ax1.max(x);
            }
        }
        if let Some(x) = row_first {
            if top.is_none() {
                top = Some((y, *gfx.get_pixel(x, y)));
            }
            ay1 = y;
        }
    }
    let (ay0, panel) = top?;
    if ax1 < ax0 {
        return None;
    }
    let (art_w, art_h) = (ax1 - ax0 + 1, ay1 - ay0 + 1);
    let (cw, ch) = (cell_px.0.max(1) as u32, cell_px.1.max(1) as u32);
    // Horizontal placement is unchanged from the band mapping: the art's native left
    // edge through the same scale. Only the VERTICAL anchor moves (centred).
    let x = band.x + ((scale.off_x as f32 + ax0 as f32 * scale.s) / cw as f32).floor() as u16;
    let mut cols = (((art_w as f32 * scale.s) / cw as f32).ceil() as u16)
        .clamp(1, band.right().saturating_sub(x).max(1));
    let mut rows = (((art_h as f32 * scale.s) / ch as f32).ceil() as u16).clamp(1, band.height);
    // Keep one column of panel fill between the picture and the divider, so the
    // panel frames the art on that side the way the art's own native left margin
    // frames it on the other. Both axes shrink by the SAME factor, so the aspect
    // ratio is untouched (the draw stretches the crop into this rect). Only applies
    // when the divider lies to the RIGHT of the art — i.e. a left-hand flank, the
    // only kind any Menu-plan game has; a right-hand flank keeps today's placement.
    if let Some(dx) = divider.map(|d| d.x).filter(|&dx| dx > x) {
        let limit = dx.saturating_sub(x).saturating_sub(1);
        if limit > 0 && cols > limit {
            let f = limit as f32 / cols as f32;
            cols = limit;
            rows = ((rows as f32 * f).round() as u16).max(1);
        }
    }
    let y = band.y + (band.height - rows) / 2;
    Some((panel, Rect::new(x, y, cols, rows), (ax0, ay0, art_w, art_h)))
}

fn flank_divider_extension(
    band: Rect,
    pane: Rect,
    viewport: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
    native: (u16, u16),
    canvas: &image::RgbaImage,
    menu_top_row: u16,
) -> Option<(Rect, (u32, u32, u32, u32))> {
    let cw = cell_px.0.max(1) as f32;
    let s = if scale.s <= 0.0 { 1.0 } else { scale.s };
    let sy0 = story.y_px as u32;
    let sy1 = (story.y_px as u32 + story.h_px as u32).min(canvas.height());
    if sy1 <= sy0 {
        return None;
    }
    // A mid-story native row: the border column is inked here regardless of where the
    // picture (which may not span the full column height) begins or ends.
    let mid = sy0 + (sy1 - sy0) / 2;
    let opaque = |x: u32, y: u32| x < canvas.width() && y < canvas.height() && canvas.get_pixel(x, y)[3] >= 128;
    // The divider/border is the contiguous opaque native column run abutting the story
    // box edge on this flank (left flank → run ending at the story's left edge; right
    // flank → run starting at the story's right edge), sampled at that mid-story row.
    let story_x0 = story.x_px as u32;
    let story_x1 = (story.x_px as u32 + story.w_px as u32).min(native.0 as u32);
    let (dnx0, dnx1) = if band.x < viewport.x {
        if story_x0 == 0 || !opaque(story_x0 - 1, mid) {
            return None;
        }
        let mut x = story_x0;
        while x > 0 && opaque(x - 1, mid) {
            x -= 1;
        }
        (x, story_x0)
    } else {
        if story_x1 >= native.0 as u32 || !opaque(story_x1, mid) {
            return None;
        }
        let mut x = story_x1;
        while x < native.0 as u32 && opaque(x, mid) {
            x += 1;
        }
        (story_x1, x)
    };
    if dnx1 <= dnx0 {
        return None;
    }
    // Device cell x-range covering the divider columns (through the uniform scale), and
    // the device row where the flank's uniform-scaled art bottoms out (the story's
    // native bottom). The extension spans from there down to the menu strip top.
    let dcell0 = pane.x + ((scale.off_x as f32 + dnx0 as f32 * s) / cw).floor() as u16;
    let dcell1 = pane.x + ((scale.off_x as f32 + dnx1 as f32 * s) / cw).ceil() as u16;
    if dcell1 <= dcell0 || menu_top_row <= band.y {
        return None;
    }
    // The divider runs the WHOLE flank column, from its top down to the menu strip.
    // It used to start where the flank art bottomed out, which was fine while the art
    // was top-anchored and carried its own divider pixels above that row. Now the art
    // is centred and cropped to its own bounding box (SQ-0547), so those pixels are no
    // longer drawn — and the divider column lies to the RIGHT of the picture, so
    // running it full height covers the gap without touching the art.
    let ext = Rect::new(dcell0, band.y, dcell1 - dcell0, menu_top_row - band.y);
    Some((ext, (dnx0, mid, dnx1 - dnx0, 1)))
}

/// Map a chrome run's native top-left game pixel to its pane-absolute terminal
/// cell (col, row) through the letterbox `scale` — the same mapping the pixel
/// ring and the inside-story overlay use, so a text strip lines up exactly with
/// the art strips beside it.
/// A v6 window's native pixel rect as terminal CELLS, both corners mapped exactly as
/// [`run_cell`] maps a run's origin: through the scale, then ROUNDED. Rounding (not
/// ceil) on the far edge is what keeps a 20px status strip — 1.25 cells — from
/// claiming a second row and eating the first line of story under it. (SQ-0584)
///
/// `row_shift` slides the whole native screen by whole terminal ROWS, for the cell
/// path's packing (SQ-0697): there the native screen is anchored on the first inked
/// chrome row rather than on the pane's top edge, and a window's erased ground has
/// to move with the glyphs painted on it. Zero everywhere else.
fn px_rect_to_cells(
    pw: &PositionedWindow,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    row_shift: i32,
) -> Rect {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    let to_col = |px: f32| pane.x as f32 + (scale.off_x as f32 + px * scale.s) / cw;
    let to_row = |py: f32| pane.y as f32 + (scale.off_y as f32 + py * scale.s) / ch + row_shift as f32;
    let x0 = to_col(pw.x_px as f32).round().max(pane.x as f32) as u16;
    let y0 = to_row(pw.y_px as f32).round().max(pane.y as f32) as u16;
    let x1 = to_col(pw.x_px.saturating_add(pw.w_px) as f32).round().max(x0 as f32).min(pane.right() as f32) as u16;
    let y1 = to_row(pw.y_px.saturating_add(pw.h_px) as f32).round().max(y0 as f32).min(pane.bottom() as f32) as u16;
    Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

fn run_cell(t: &crate::engine::PxText, scale: &crate::render::v6_layout::Scale, cell_px: (u16, u16), pane: Rect) -> (i32, i32) {
    let cw = cell_px.0.max(1) as f32;
    let ch = cell_px.1.max(1) as f32;
    let px = t.x.max(1) as f32 - 1.0;
    let py = t.y.max(1) as f32 - 1.0;
    let col = pane.x as i32 + ((scale.off_x as f32 + px * scale.s) / cw).round() as i32;
    let row = pane.y as i32 + ((scale.off_y as f32 + py * scale.s) / ch).round() as i32;
    (col, row)
}

/// Carve the hybrid chrome `bands` into drawable strips (SQ-0500). Narrow side
/// bands (beside the story viewport) stay one `Art` strip — picture columns and
/// borders. Each FULL-WIDTH band (top/bottom of the ring) is split row-by-row:
/// a terminal row is a TEXT row when it carries chrome runs that lie OUTSIDE the
/// story box, above or below it, with NO opaque frame graphics behind them; every
/// other row is ART. Consecutive rows of one class merge into a strip. `story` is
/// the story window (its native pixel box splits above/below); `gfx` is the
/// graphics-only chrome canvas (the art test, via [`region_has_opaque`]).
///
/// `panels` are the cell rects of SECONDARY PROSE windows (SQ-0585) — a v6 game's
/// second scrolling text window, which the renderer draws as terminal text of its
/// own. Those rows belong to that window, so no strip is emitted for them at all:
/// classing them ART made the ring rasterize a slice of the chrome canvas straight
/// over the panel, and under a graphics protocol like kitty the image composites
/// ABOVE the cells, so the panel's text vanished behind stray rasterized banner.
fn decompose_chrome_strips<'a>(
    bands: &[Rect],
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
    overlay_bottom: i32,
    panels: &[Rect],
    gfx: &image::RgbaImage,
    runs: &[&'a crate::engine::PxText],
) -> Vec<ChromeStrip<'a>> {
    use crate::render::v6_layout::region_has_opaque;
    let story_top = story.y_px as i32;
    let story_bottom = story.y_px as i32 + story.h_px as i32;
    // A run sits over opaque frame art when its glyph span overlaps graphics.
    let over_art = |t: &crate::engine::PxText| -> bool {
        let px0 = t.x.max(1) as u32 - 1;
        let py = t.y.max(1) as u32 - 1;
        let w = t.text.chars().count().max(1) as u32 * 8;
        region_has_opaque(gfx, px0, py, w, 16)
    };
    // A run is a text-band candidate when it lies fully above or below the story
    // box (never beside it — those stay in the side bands' ring).
    // …plus a status strip the game OVERLAYS on the story box (SQ-0582): its runs are
    // inside the box by native coordinates, but the caller has reserved their rows out
    // of the story viewport, so they belong to the band like any other bar.
    let below_or_above = |t: &crate::engine::PxText| -> bool {
        let py = t.y.max(1) as i32 - 1;
        py >= story_bottom || py + 16 <= story_top || (overlay_bottom > 0 && py + 16 <= overlay_bottom)
    };
    let row_runs_at = |row: u16| -> Vec<&'a crate::engine::PxText> {
        runs.iter()
            .copied()
            .filter(|t| below_or_above(t) && run_cell(t, scale, cell_px, pane).1 == row as i32)
            .collect()
    };
    // One terminal row's class within a full-width band.
    enum RowClass<'b> {
        Text(Vec<&'b crate::engine::PxText>),
        Art,
        /// No runs and no opaque frame art behind — bare background.
        Empty,
        /// A secondary prose window owns this row; the ring must not draw here.
        Panel,
    }
    let in_panel = |row: u16| panels.iter().any(|p| row >= p.y && row < p.bottom());
    let mut out = Vec::new();
    for band in bands {
        // Side bands (narrower than the pane) are never text — one Art strip.
        if band.width < pane.width {
            out.push(ChromeStrip::Art(*band));
            continue;
        }
        // Classify each terminal row of this full-width band.
        let mut classes: Vec<RowClass> = Vec::new();
        for row in band.y..band.bottom() {
            if in_panel(row) {
                classes.push(RowClass::Panel);
                continue;
            }
            let rr = row_runs_at(row);
            classes.push(if rr.is_empty() {
                RowClass::Empty
            } else if rr.iter().any(|t| over_art(t)) {
                RowClass::Art
            } else {
                RowClass::Text(rr)
            });
        }
        // SQ-0508: bridge a scale-introduced interior gap row into the menu panel.
        // When the letterbox scale spreads N native menu rows across N+ terminal
        // rows, a bare terminal row can fall BETWEEN two menu rows (Journey's
        // command menu: a blank row below the header, and one above "Tag"), breaking
        // the reversed vertical column dividers. An Empty row whose nearest non-Empty
        // neighbour above AND below are both Text is part of that panel → mark it Text
        // so the whole menu is one strip (continuous background + dividers). Empty
        // rows at an art boundary (Arthur's panel over the status) stay Art, so the
        // pixel ring keeps showing through there.
        let n = classes.len();
        let is_text = |c: &RowClass| matches!(c, RowClass::Text(_));
        let mut bridge = vec![false; n];
        for i in 0..n {
            if !matches!(classes[i], RowClass::Empty) {
                continue;
            }
            let _ = &classes[i];
            let above = (0..i).rev().find(|&j| !matches!(classes[j], RowClass::Empty));
            let below = (i + 1..n).find(|&j| !matches!(classes[j], RowClass::Empty));
            if above.is_some_and(|j| is_text(&classes[j])) && below.is_some_and(|j| is_text(&classes[j])) {
                bridge[i] = true;
            }
        }
        // Coalesce consecutive same-class (Text|bridged vs. not) rows into strips.
        let mut i = 0usize;
        while i < n {
            // A panel's rows produce no strip: that window draws itself.
            if matches!(classes[i], RowClass::Panel) {
                i += 1;
                continue;
            }
            let text = matches!(classes[i], RowClass::Text(_)) || bridge[i];
            let mut j = i;
            let mut text_runs: Vec<&crate::engine::PxText> = Vec::new();
            while j < n
                && !matches!(classes[j], RowClass::Panel)
                && (matches!(classes[j], RowClass::Text(_)) || bridge[j]) == text
            {
                if let RowClass::Text(rr) = &classes[j] {
                    text_runs.extend(rr.iter().copied());
                }
                j += 1;
            }
            let rect = Rect::new(band.x, band.y + i as u16, band.width, (j - i) as u16);
            out.push(if text { ChromeStrip::Text(rect, text_runs) } else { ChromeStrip::Art(rect) });
            i = j;
        }
    }
    out
}

/// Paint a TEXT chrome strip (SQ-0500) as terminal cells: each run stamped at its
/// scale-mapped cell with [`v6_run_style`], clipped to `rect`. The strip and each
/// run row are flooded (colour-aware, SQ-0512) before the runs stamp, so the panel
/// reads as one solid block carrying the game's own background — not just cells
/// behind the glyphs. A PURE reverse-video row (a status/menu bar — every run
/// reversed) floods edge to edge reversed, so a bar the game painted as separate
/// runs with bare gaps reads as one solid block (SQ-0499 cell path): Arthur's
/// status row loses its lone unreversed cell; Journey's menu header bar closes the
/// gap between its two labels. Mixed rows (Journey's menu body — normal verb text
/// with reversed dividers) are not flood-reversed.
#[allow(clippy::too_many_arguments)]
fn draw_chrome_text_strip(
    runs: &[&crate::engine::PxText],
    rect: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
    base: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
    buf: &mut Buffer,
) {
    use crate::engine::PxText;
    use std::collections::BTreeMap;

    // SQ-0508(a)/SQ-0512: fill the WHOLE strip first, so the menu/status panel reads
    // as one solid block — the cells around and between the runs no longer show the
    // theme backdrop. The fill is COLOUR-AWARE: resolve the first run in the strip
    // that set an explicit game colour (per channel) and flood with THAT bg/fg over
    // the themed `base`, so a game (Shogun) that paints its status band with an
    // explicit background floods the whole band, not just the glyph cells, and the
    // blank/bridged gap rows between run rows read as part of the same panel. When no
    // run sets an explicit colour this is byte-identical to a bare `base` flood
    // (Journey's black menu panel, Arthur's status strip). `base` is the
    // `upper_window` theme style; a per-run explicit colour still wins where the run
    // stamps over this fill below.
    let strip_fg = runs.iter().map(|t| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
    let strip_bg = runs.iter().map(|t| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
    let strip_fill = if crate::render::v6_layout::packed_explicit(strip_fg)
        || crate::render::v6_layout::packed_explicit(strip_bg)
    {
        v6_run_style(base, strip_fg, strip_bg, 0, honor, colors)
    } else {
        base
    };
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(strip_fill);
            }
        }
    }

    // Bucket runs by their GAME text row, laid out on CONSECUTIVE terminal rows
    // from the strip's top (SQ-0543).
    //
    // The chrome ring's ART scales with the pane, but terminal TEXT does not —
    // it is always one terminal cell tall. So the taller the pane, the more
    // terminal rows one 16px game row spans, and at a large pane two adjacent
    // status lines map two rows apart: Shogun's two-line band grew a blank row
    // straight through its middle. Inside a TEXT strip there is no art to stay
    // aligned with — having no frame graphics behind it is what MAKES it a text
    // strip — so the game's own row structure is the truth to preserve, not the
    // device-pixel position.
    //
    // The strip begins at its first text row (`decompose_chrome_strips` carves
    // it that way), so offsetting each run's game row from the topmost one lands
    // the first row exactly where it does today. Genuinely blank game rows
    // survive, since their indices differ by more than one; and wherever the old
    // mapping already produced consecutive rows — any pane small enough that a
    // game row is about a terminal row — the result is byte-identical.
    const FONT_H: i32 = 16; // the v6 text cell is 8×16 (SQ-0479)
    let game_row = |t: &PxText| (t.y.max(1) as i32 - 1) / FONT_H;
    let first_row = runs.iter().map(|t| game_row(t)).min().unwrap_or(0);
    let mut raw: BTreeMap<i32, Vec<&PxText>> = BTreeMap::new();
    for t in runs {
        raw.entry(rect.y as i32 + game_row(t) - first_row).or_default().push(t);
    }
    // SQ-0509: merge horizontally-contiguous same-style fragments before mapping.
    // Runs separated by a genuine gap (Journey's menu items / column dividers,
    // 8px apart) stay distinct and keep their proportional spacing, so the strip
    // bridges only ABUTTING fragments. SQ-0742 first collapses each repeated-glyph
    // RULE to the width of its own scaled span, and flags it, so the stamping below
    // can close the seams around it (see `collapse_row_rules`).
    let mut by_row: BTreeMap<i32, Vec<(PxText, bool)>> = BTreeMap::new();
    for (row, mut rr) in raw {
        rr.sort_by_key(|t| t.x);
        by_row.insert(row, collapse_row_rules(&rr, scale, cell_px, pane));
    }

    // SQ-0508(b): divider columns to draw continuously. A reversed WHITESPACE run in
    // a MIXED row (normal verb text among reversed dividers — Journey's menu body) is
    // a vertical column divider; extend every such column across the FULL strip height
    // so the scale-introduced gap rows (bridged in as blank Text rows) don't break the
    // lines. Collected from mixed rows only, so a pure-reverse bar row (a header /
    // status bar, already filled edge to edge below) contributes none.
    let mut divider_cols: Vec<u16> = Vec::new();
    for row_runs in by_row.values() {
        let mixed = row_runs.iter().any(|(t, _)| t.style & 1 == 0);
        if !mixed {
            continue;
        }
        for (t, _) in row_runs {
            if t.style & 1 != 0 && t.text.trim().is_empty() {
                let (c, _) = run_cell(t, scale, cell_px, pane);
                if c >= rect.x as i32 && c < rect.right() as i32 {
                    divider_cols.push(c as u16);
                }
            }
        }
    }
    divider_cols.sort_unstable();
    divider_cols.dedup();
    if !divider_cols.is_empty() {
        let rev = v6_run_style(base, 0, 0, 1, honor, colors);
        for y in rect.y..rect.bottom() {
            for &c in &divider_cols {
                buf.set_stringn(c, y, " ", 1, rev);
            }
        }
    }

    for (row, row_runs) in &by_row {
        if *row < rect.y as i32 || *row >= rect.bottom() as i32 {
            continue;
        }
        // SQ-0512: flood this row's FULL strip width before stamping its runs, so the
        // row reads as one solid panel — the cells around AND between the runs carry
        // the row's own background, not the theme backdrop. The fill colour is the
        // first run in the row with an explicit game colour, per channel, over `base`
        // (Shogun's status band floods its explicit white edge to edge). A PURE
        // reverse-video row (a bar the game draws edge to edge — Arthur's status row,
        // Journey's menu header) floods reversed, subsuming the old pure-reverse gap
        // fill (SQ-0504): the runs re-stamp reversed over it, so a full-width band
        // spans the whole pane. A MIXED row (Journey's menu body — normal verbs among
        // reversed dividers) is NOT flood-reversed; its reversed divider runs re-stamp
        // over an un-reversed flood below. Colourless non-reverse rows keep the strip
        // `base` flood untouched (byte-identical), so Journey's menu body is unchanged.
        let all_rev = !row_runs.is_empty() && row_runs.iter().all(|(t, _)| t.style & 1 != 0);
        let row_fg = row_runs.iter().map(|(t, _)| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_bg = row_runs.iter().map(|(t, _)| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        if all_rev
            || crate::render::v6_layout::packed_explicit(row_fg)
            || crate::render::v6_layout::packed_explicit(row_bg)
        {
            let fill = v6_run_style(base, row_fg, row_bg, all_rev as u8, honor, colors);
            for c in rect.x..rect.right() {
                buf.set_stringn(c, *row as u16, " ", 1, fill);
            }
        }
        // SQ-0727: the cells this row's GLYPH runs occupy, so a blank run cannot
        // erase one. A run is POSITIONED by its scale-mapped native pixel, but its
        // characters then advance ONE TERMINAL COLUMN each — two different rates
        // wherever the pane is not exactly one column per native 8px text cell (at
        // 120 columns of a 640px screen it is one and a half). So a blank run the
        // game painted over another run's OWN whitespace no longer lands on that
        // whitespace once mapped: it lands on a neighbouring glyph.
        //
        // advent.z6's help screen is the report. It paints each bar row as one
        // label run plus the reversed blank cells of the bar around and between the
        // labels, and at 120 columns the blanks at native x=17/33/73 mapped onto
        // "N = next subject"'s columns 3/6/14 — its `=` and both lowercase `e`s —
        // while the blank at x=113, one native cell past the label's last character,
        // mapped INSIDE its cell span and clipped the tail off "RETURN = read
        // subjec[t]". Interior drops and a clipped tail, one mechanism. At 80
        // columns the two rates coincide and the row was always correct, which is
        // why this reads as a font bug and is a scale bug.
        //
        // A blank run carries no glyphs: the strip and row floods above already put
        // its background down, and in NATIVE pixels it only ever covers whitespace
        // the glyph run drew itself. So it may still paint the cells no glyph run
        // claimed (the bar's own gaps), and must skip the rest.
        //
        // SQ-0742: a RULE ([`collapse_row_rules`]) closes the seams a scale leaves
        // around it — it runs from the end of whatever is drawn before it to the
        // start of whatever comes after, so a border reads as one unbroken line
        // through its own corners and titles instead of a rule with a hole either
        // side of every neighbour. Everything else keeps exactly the span its
        // characters occupy.
        let base_span = |t: &PxText| {
            let (c, _) = run_cell(t, scale, cell_px, pane);
            (c, c + t.text.chars().count() as i32)
        };
        let mut spans: Vec<(i32, i32)> = Vec::with_capacity(row_runs.len());
        for (i, (t, rule)) in row_runs.iter().enumerate() {
            let (c0, c1) = base_span(t);
            spans.push(if *rule {
                let left = spans.last().map_or(c0, |&(_, prev_end)| prev_end);
                let right = row_runs.get(i + 1).map_or(c1, |n| base_span(&n.0).0);
                (left, right.max(left))
            } else {
                (c0, c1)
            });
        }
        let claimed: Vec<(i32, i32)> = row_runs
            .iter()
            .zip(&spans)
            .filter(|((t, _), _)| !t.text.trim().is_empty())
            .map(|(_, &s)| s)
            .collect();
        let is_claimed = |c: i32| claimed.iter().any(|&(lo, hi)| c >= lo && c < hi);
        for ((t, rule), &(col, end)) in row_runs.iter().zip(&spans) {
            if col < rect.x as i32 || col >= rect.right() as i32 {
                continue;
            }
            let style = v6_run_style(base, t.fg, t.bg, t.style, honor, colors);
            let max_w = rect.right() as usize - col as usize;
            if max_w == 0 {
                continue;
            }
            if let Some(g) = rule.then(|| t.text.chars().next()).flatten() {
                let text: String = std::iter::repeat_n(g, (end - col).max(0) as usize).collect();
                buf.set_stringn(col as u16, *row as u16, &text, max_w, style);
                continue;
            }
            // Untrusted game text (SQ-0639).
            let text = crate::render::blank_control_chars(&t.text);
            if t.text.trim().is_empty() {
                for (i, ch) in text.chars().take(max_w).enumerate() {
                    let c = col + i as i32;
                    if !is_claimed(c) {
                        buf.set_stringn(c as u16, *row as u16, ch.encode_utf8(&mut [0u8; 4]), 1, style);
                    }
                }
            } else {
                buf.set_stringn(col as u16, *row as u16, text.as_ref(), max_w, style);
            }
        }
    }
}

/// SQ-0742: collapse each repeated-glyph RULE in one native text row to the width
/// of its own SCALED span, then merge what is left with [`merge_row_fragments`].
///
/// A text strip POSITIONS a run through the letterbox scale but then advances ONE
/// TERMINAL COLUMN per character — the two rates only coincide where the pane is
/// exactly one column per native 8px text cell. For a label that is exactly right:
/// prose has to stay legible, so its character count is what it is. For a RULE it
/// is wrong, because a rule is a *distance* the game drew across, not a string of
/// that many characters. Journey under the Amiga interpreter draws its whole frame
/// that way — `┌`, seventy-eight `─` fragments, `┐` — and one cell per fragment
/// stopped the border at column 79 of a 138-column pane while the prose beside it
/// wrapped to the pane. The same frame under the IBM PC profile is reverse-video
/// SPACES, which the row flood already spreads edge to edge, which is why only the
/// Amiga route ever showed it.
///
/// A rule is [`RULE_MIN`] or more ABUTTING fragments, each a single SYMBOL glyph
/// (never a letter or digit), all at the same style and colours. Each such group
/// becomes one run repeating that glyph across the cells its native span maps to,
/// and is kept OUT of the fragment merge so an adjoining corner or title cannot
/// glue itself on and drag the row back to one cell per native character.
///
/// The predicate is deliberately narrow, because the fragments it reads are the
/// same ones SQ-0509 exists to reassemble: a game with proportional metrics emits
/// one run per GLYPH, so "Anne" arrives as `A` `n` `n` `e` and a rule test of "two
/// abutting equal fragments" reads every doubled letter in the corpus as a rule —
/// Arthur's status bar lost its character's name to exactly that. Requiring a
/// non-alphanumeric glyph and three of them in a row leaves prose alone while
/// still catching every frame rule, which no game draws in two segments.
///
/// Blank runs are left alone as well: the strip and row floods already spread a
/// reverse-video bar edge to edge, so skipping them keeps every game that draws
/// its chrome that way byte-identical.
fn collapse_row_rules(
    row_runs: &[&crate::engine::PxText],
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    pane: Rect,
) -> Vec<(crate::engine::PxText, bool)> {
    use crate::engine::PxText;
    /// Fewest abutting fragments that count as a rule rather than as prose.
    const RULE_MIN: usize = 3;
    // Is `t` one glyph the game could have repeated into a rule?
    let single_glyph = |t: &PxText| -> Option<char> {
        let mut cs = t.text.chars();
        match (cs.next(), cs.next()) {
            (Some(c), None) if !c.is_whitespace() && !c.is_alphanumeric() => Some(c),
            _ => None,
        }
    };
    let end_px = |t: &PxText| (t.x.max(1) as i32 - 1) + t.text.chars().count() as i32 * 8;
    let mut out: Vec<(PxText, bool)> = Vec::new();
    // Pending non-rule fragments, merged together on the far side of each rule so
    // SQ-0509's fragment bridging is unchanged everywhere a rule is not involved.
    let mut pending: Vec<&PxText> = Vec::new();
    let mut i = 0usize;
    while i < row_runs.len() {
        let t = row_runs[i];
        // How far does a run of this same glyph, abutting, continue?
        let mut j = i;
        if let Some(g) = single_glyph(t) {
            while j + 1 < row_runs.len() {
                let n = row_runs[j + 1];
                if single_glyph(n) == Some(g)
                    && (n.x.max(1) as i32 - 1) == end_px(row_runs[j])
                    && n.style == t.style
                    && n.fg == t.fg
                    && n.bg == t.bg
                {
                    j += 1;
                } else {
                    break;
                }
            }
        }
        if j + 1 - i >= RULE_MIN {
            // A rule. Flush what came before it, then emit it at its scaled width.
            out.extend(merge_row_fragments(&pending, 4).into_iter().map(|t| (t, false)));
            pending.clear();
            let (col0, _) = run_cell(t, scale, cell_px, pane);
            let cw = cell_px.0.max(1) as f32;
            let end_dev = scale.off_x as f32 + end_px(row_runs[j]) as f32 * scale.s;
            let col1 = pane.x as i32 + (end_dev / cw).round() as i32;
            let cells = (col1 - col0).max(1) as usize;
            let mut rule = t.clone();
            rule.text = std::iter::repeat_n(single_glyph(t).expect("checked above"), cells).collect();
            out.push((rule, true));
            i = j + 1;
            continue;
        }
        pending.push(t);
        i += 1;
    }
    out.extend(merge_row_fragments(&pending, 4).into_iter().map(|t| (t, false)));
    out
}

/// SQ-0509: merge horizontally-contiguous same-style fragments of ONE native text
/// row (`row_runs` sorted by `x`) into single runs. A game that positions status
/// text with proportional pixel metrics — Arthur — emits word fragments as
/// separate runs whose pixel start abuts the previous run's pixel end; placing
/// each fragment independently scatters them ("Chu rch yard", or one anchor group
/// per glyph). A run starting within `tol_px` of the previous run's end, with
/// identical style and colours, is concatenated onto it, the intervening pixel gap
/// becoming `gap / 8` spaces — so a `tol_px` of 4 bridges only ABUTTING fragments
/// (adding nothing) while 8 also closes a one-cell word gap with a real space.
/// Runs separated by a wider gap stay distinct and keep their own positions.
fn merge_row_fragments(row_runs: &[&crate::engine::PxText], tol_px: i32) -> Vec<crate::engine::PxText> {
    let mut merged: Vec<crate::engine::PxText> = Vec::new();
    for t in row_runs {
        if let Some(last) = merged.last_mut() {
            let last_end = (last.x.max(1) as i32 - 1) + last.text.chars().count() as i32 * 8;
            let start = t.x.max(1) as i32 - 1;
            if start >= last_end
                && start - last_end <= tol_px
                && last.style == t.style
                && last.fg == t.fg
                && last.bg == t.bg
            {
                for _ in 0..(start - last_end) / 8 {
                    last.text.push(' ');
                }
                last.text.push_str(&t.text);
                continue;
            }
        }
        merged.push((*t).clone());
    }
    merged
}

/// Render the v6 frameless status band as a classic full-width status line
/// ("anchored bar", SQ-0467). `runs` are all the chrome grids' pixel-text runs;
/// `ncols` is the native screen width in cells (so anchor thresholds scale to the
/// game's own screen, not a hardcoded 40). Each native row (`(y-1)/16`) below
/// `band_rows` is classified into LEFT/CENTER/RIGHT anchor groups and painted
/// across the full pane width. Returns the number of band rows used (for the
/// story offset).
///
/// `band_rows` is the story window's TOP native row, not a constant (SQ-0549):
/// the status band is whatever chrome text sits ABOVE the story, wherever the
/// game put it. The band is ANCHORED to the pane top — its first inked native row
/// draws at `area.y`, and the rows below keep their relative spacing — so Arthur's
/// row-12 bar (its story buffer starts at row 13, under a 12-row art panel that
/// frameless mode drops) reads as a top status line instead of floating a quarter
/// of the way down the pane.
/// How many pane rows [`draw_anchored_status_band`] will use, without painting
/// anything (SQ-0712). The band has to be measured before the story area can be
/// sized and painted after the erase fills, so the two are split: this is the
/// span from the first inked native row inside the band to the last, clamped to
/// the pane, which is exactly what the draw returns.
fn anchored_band_rows(runs: &[&crate::engine::PxText], band_rows: u16, pane_h: u16) -> u16 {
    let inked = || {
        runs.iter()
            .filter(|t| !t.text.trim().is_empty())
            .map(|t| (t.y.max(1) - 1) / 16)
            .filter(|&r| r < band_rows)
    };
    let Some(first) = inked().min() else { return 0 };
    // The draw stops at the pane's bottom edge, so a row that would land off-pane
    // never paints and never counts — the band must stay in-pane either way.
    inked()
        .filter(|&r| r - first < pane_h)
        .max()
        .map(|last| last - first + 1)
        .unwrap_or(0)
}

fn draw_anchored_status_band(
    runs: &[&crate::engine::PxText],
    ncols: u32,
    band_rows: u16,
    area: Rect,
    buf: &mut Buffer,
    style: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
) -> u16 {
    let left_bound = ncols / 3; // left-third boundary (cells)
    let right_bound = ncols * 2 / 3; // right two-thirds boundary (cells)
    // The band's own origin: the topmost inked native row inside it.
    let Some(first_row) = runs
        .iter()
        .filter(|t| !t.text.trim().is_empty())
        .map(|t| (t.y.max(1) - 1) / 16)
        .filter(|&r| r < band_rows)
        .min()
    else {
        return 0;
    };
    let mut rows_used = 0u16;
    for row in first_row..band_rows {
        if area.y + (row - first_row) >= area.bottom() {
            break; // the band must stay in-pane
        }
        // This native row's non-blank runs, across ALL chrome grids, left→right.
        let mut row_runs: Vec<&crate::engine::PxText> = runs
            .iter()
            .copied()
            .filter(|t| !t.text.trim().is_empty() && (t.y.max(1) - 1) / 16 == row)
            .collect();
        if row_runs.is_empty() {
            continue;
        }
        row_runs.sort_by_key(|t| t.x);
        // Glue the row's word fragments back together before classifying (SQ-0509,
        // reached here by SQ-0549): Arthur paints its bar one GLYPH per run, which
        // would otherwise put every letter in its own anchor group and join them
        // with two spaces apiece. The 8px tolerance also restores the single-cell
        // word gaps inside its date field ("St Anne's Day, Compline"), which a
        // group join would likewise have doubled.
        let row_runs = merge_row_fragments(&row_runs, 8);
        // Classify each run into an anchor group by its native position. A run
        // the game CENTRED on its own screen is CENTER wherever it starts (see
        // below); otherwise a run spanning most of the row (a full-width bar)
        // counts LEFT, a start in the left third is LEFT, an end past the right
        // two-thirds is RIGHT, and everything between is CENTER. Within a group,
        // run order is preserved and the native gaps collapse to a two-space join.
        let (mut left, mut center, mut right): (Vec<&str>, Vec<&str>, Vec<&str>) =
            (Vec::new(), Vec::new(), Vec::new());
        for t in &row_runs {
            let start = ((t.x.max(1) - 1) / 8) as u32;
            let len = t.text.chars().count() as u32;
            let end = start + len;
            // SQ-0717: the thirds rule reads a run's START, which is the right
            // question for a status FIELD (a location name begins at the left
            // margin, a score ends at the right one) and the wrong one for a line
            // the game centred by cursor arithmetic. Shogun's frozen title header
            // is nine such lines (SQ-0697) — the longer ones begin left of the
            // left-third boundary and the shortest ends past the right two-thirds,
            // so five of nine were flushed to col 0 and one flushed right, wrecking
            // the block the game had carefully centred. A run with equal margins on
            // its own screen — within the one cell that 8px column quantization can
            // cost — was centred deliberately, so it is CENTER, and the pane centres
            // it again at whatever width the terminal happens to be. Both margins
            // must be non-zero: a rule or bar drawn from the screen edge is not
            // centred text, and stays the LEFT-anchored bar it was.
            let centred =
                start > 0 && end < ncols && start.abs_diff(ncols - end) <= 1;
            if centred {
                center.push(&t.text);
            } else if len * 3 >= ncols * 2 || start < left_bound {
                left.push(&t.text);
            } else if end > right_bound {
                right.push(&t.text);
            } else {
                center.push(&t.text);
            }
        }
        let left_str = left.join("  ");
        let center_str = center.join("  ");
        let right_str = right.join("  ");
        // Resolve this band row's style from its runs (SQ-0488): the first run
        // that set an explicit game colour contributes that channel over the
        // themed base, and any reversed run flips the row — so Zork0's dark-on-
        // tan ribbon labels keep their colours while Shogun's Default/Default
        // runs stay exactly the theme style. The band collapses multiple runs
        // into left/center/right strings painted with one style, so per-channel
        // is first-explicit-wins across the row rather than per-substring.
        let row_fg = row_runs.iter().map(|t| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_bg = row_runs.iter().map(|t| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_rev = row_runs.iter().any(|t| t.style & 1 != 0) as u8;
        let row_style = v6_run_style(style, row_fg, row_bg, row_rev, honor, colors);
        if place_anchored_row(buf, area, area.y + (row - first_row), &left_str, &center_str, &right_str, row_style) {
            rows_used = rows_used.max(row - first_row + 1);
        }
    }
    rows_used
}

/// Paint one anchored status row across the full pane width: LEFT flush at col 0,
/// RIGHT flush to the last column, CENTER centered. Overlap priority (narrow
/// panes): LEFT wins; RIGHT truncates from its left edge to keep ≥1 space from
/// LEFT; CENTER drops entirely if it can't fit between them with a space each
/// side. Never overwrites one group with another; never panics on width 1–2.
/// Returns whether anything was painted.
fn place_anchored_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    left: &str,
    center: &str,
    right: &str,
    style: ratatui::style::Style,
) -> bool {
    let w = area.width as usize;
    if w == 0 {
        return false;
    }
    // Untrusted game text (SQ-0639): blank control chars before any of it reaches
    // the buffer. Blanking is char-for-char, so every width/anchor computation
    // below is unchanged.
    let (left_txt, center_txt, right_txt) = (
        crate::render::blank_control_chars(left),
        crate::render::blank_control_chars(center),
        crate::render::blank_control_chars(right),
    );
    let (left, center, right) = (left_txt.as_ref(), center_txt.as_ref(), right_txt.as_ref());
    let mut painted = false;

    // Fill the WHOLE band row with the status style's background first, so the
    // band reads as one solid bar (the upper_window bg fills the gaps between
    // the anchored groups), not just coloured cells behind the glyphs (SQ-0467
    // follow-up: fill first, stamp runs after).
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }

    // LEFT — flush at col 0, truncated to the pane width.
    let left_len = left.chars().count().min(w);
    if left_len > 0 {
        buf.set_stringn(area.x, y, left, w, style);
        painted = true;
    }

    // RIGHT — flush to the last column; truncate leading chars if it would collide
    // with LEFT (keeping ≥1 space between). Truncating from the left keeps the end
    // flush right.
    let min_right_start = if left_len > 0 { left_len + 1 } else { 0 };
    let mut right_str: String = right.to_string();
    let mut right_len = right_str.chars().count();
    if right_len > 0 {
        let avail = w.saturating_sub(min_right_start);
        if right_len > avail {
            let drop = right_len - avail;
            right_str = right_str.chars().skip(drop).collect();
            right_len = right_str.chars().count();
        }
        if right_len > 0 {
            let right_start = w - right_len;
            buf.set_stringn(area.x + right_start as u16, y, &right_str, right_len, style);
            painted = true;
        }
    }

    // CENTER — centered, but only if it fits in the gap between LEFT and RIGHT with
    // a space on each side; otherwise dropped entirely.
    let center_len = center.chars().count();
    if center_len > 0 {
        let gap_lo = if left_len > 0 { left_len + 1 } else { 0 };
        let gap_hi = if right_len > 0 { (w - right_len).saturating_sub(1) } else { w };
        if gap_hi > gap_lo && center_len <= gap_hi - gap_lo {
            let natural = w.saturating_sub(center_len) / 2;
            let start = natural.clamp(gap_lo, gap_hi - center_len);
            buf.set_stringn(area.x + start as u16, y, center, center_len, style);
            painted = true;
        }
    }

    painted
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{GridWindow, Split};
    use crate::state::StyleRun;
    use ratatui::layout::Rect;

    /// Real-Zork0 raster acceptance (SQ-0510): compose the raster canvas exactly
    /// as the raster branch does, then prove the finished image is fully opaque,
    /// that the story page and the ink are distinct, and that not one artwork
    /// pixel was painted over. Skips cleanly when the gitignored story is absent.
    #[test]
    fn zork0_raster_canvas_is_opaque_and_preserves_art() {
        use crate::render::v6_layout as v6;
        let story_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/zork0-r393-s890714.z6");
        let Ok(bytes) = std::fs::read(&story_path) else {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return;
        };
        let mut picts = crate::graphics::PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
        let dims = picts.all_pict_dims();
        let mut session =
            crate::session::GameSession::new_with_trace(bytes, false, false, None, false, dims, picts.std_window(), None, None)
                .expect("Zork0 (v6) loads and boots");
        session.set_pict_source(Some(picts));
        session.flush_boot_pictures();
        let model = crate::engine::Engine::screen(&session);
        let items = match &model.root {
            WinNode::Layered(v) => v.clone(),
            other => panic!("expected Layered, got {other:?}"),
        };

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript("West of House");
        state.push_transcript("You are standing in an open field west of a white house.");

        // The user's dark terminal: OSC 10/11 both answered → the pair is the
        // terminal's own light ink on its dark page.
        let osc = crate::term_colors::TermDefaultColors {
            fg: Some(image::Rgba([216, 216, 216, 255])),
            bg: Some(image::Rgba([26, 26, 26, 255])),
        };
        let (ink, page_default) =
            v6_default_pair(state.colors.theme.get("transcript").style, osc.fg, osc.bg);
        assert_ne!(ink, page_default, "ink and page must never resolve to the same colour");

        // ── Compose exactly as the raster branch does ────────────────────────
        let native = v6::native_extent(&items);
        let layout = v6::classify_windows(&items);
        let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, ink, page_default, &state.colors);
        let chrome_only = canvas.clone(); // pre-fill artwork reference
        let page = v6::story_bg_rgba(layout.story, &state.colors).unwrap_or(page_default);
        let (sx, sy, sw, sh) = v6::story_clear_native(layout.story, &canvas).expect("Zork0 has a story window");
        assert!(sw > 0 && sh > 0, "Zork0's clear story interior is non-empty: {sw}x{sh}");
        v6::fill_cell(&mut canvas, sx, sy, sw, sh, page);
        let cols = (sw / 8).max(1) as u16;
        let rows = (sh / 16).max(1) as u16;
        let (main, _) = build_main_text(&state, cols, rows);
        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, ink, &[]);
        let pre_flatten = canvas.clone();
        v6::flatten_onto_page(&mut canvas, page);

        // (1) The shipped image is fully opaque: no pixel is left for a compositor
        // (kitty's terminal backdrop, halfblocks' white `Color::Reset`) to resolve.
        assert!(
            canvas.pixels().all(|p| p[3] == 255),
            "every pixel of the raster composite is opaque"
        );

        // (2) Nothing already drawn was painted over — every pixel any layer had
        // touched is byte-identical after the flatten (frame art, banner text,
        // status bands, glyphs, and any inline drop-cap alike).
        for (x, y, p) in pre_flatten.enumerate_pixels() {
            if p[3] > 0 {
                assert_eq!(canvas.get_pixel(x, y), p, "flatten must not repaint drawn pixel ({x},{y})");
            }
        }
        // ...and the frame artwork specifically still matches the pre-fill chrome.
        let mut art_pixels = 0u32;
        for (x, y, p) in chrome_only.enumerate_pixels() {
            if p[3] > 0 {
                art_pixels += 1;
                assert_eq!(canvas.get_pixel(x, y), p, "frame art pixel ({x},{y}) survives fill+flatten");
            }
        }
        assert!(art_pixels > 10_000, "Zork0's frame art is substantial: {art_pixels} px");

        // (3) The story interior reads as the resolved page, and the text on it is
        // visible (glyph ink differs from the page).
        assert_eq!(*canvas.get_pixel(sx + sw / 2, sy + sh / 2), page, "story interior is the opaque page");
        let ink_px = canvas
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                (sx..sx + sw).contains(x) && (sy..sy + sh).contains(y) && **p == ink
            })
            .count();
        assert!(ink_px > 100, "seeded story text is drawn in ink on the page: {ink_px} ink pixels");
    }

    #[test]
    fn v6_default_pair_resolves_ink_and_page_from_one_source() {
        use ratatui::style::{Color, Style};
        let osc_fg = Some(image::Rgba([10, 20, 30, 255]));
        let osc_bg = Some(image::Rgba([40, 50, 60, 255]));
        let osc_pair = (image::Rgba([10, 20, 30, 255]), image::Rgba([40, 50, 60, 255]));
        let fallback = (RASTER_FALLBACK_INK, RASTER_FALLBACK_PAGE);

        // (a) Theme supplies BOTH channels → the theme pair, OSC ignored.
        let both = Style::default().fg(Color::Rgb(1, 2, 3)).bg(Color::Rgb(4, 5, 6));
        assert_eq!(
            v6_default_pair(both, osc_fg, osc_bg),
            (image::Rgba([1, 2, 3, 255]), image::Rgba([4, 5, 6, 255]))
        );

        // (b) THE REGRESSION: theme supplies fg ONLY (a cream ink with no page) and
        // OSC answered both → the OSC pair, NOT the theme ink mixed with an OSC page.
        let fg_only = Style::default().fg(Color::Rgb(1, 2, 3));
        assert_eq!(v6_default_pair(fg_only, osc_fg, osc_bg), osc_pair);
        // Symmetric partiality: theme supplies bg ONLY → still skipped whole.
        let bg_only = Style::default().bg(Color::Rgb(4, 5, 6));
        assert_eq!(v6_default_pair(bg_only, osc_fg, osc_bg), osc_pair);

        // (c) No theme RGB at all + OSC answered both → the OSC pair.
        let unset = Style::default();
        assert_eq!(v6_default_pair(unset, osc_fg, osc_bg), osc_pair);

        // (d) Only ONE OSC channel answered → the fallback pair (no mixing).
        assert_eq!(v6_default_pair(unset, osc_fg, None), fallback);
        assert_eq!(v6_default_pair(unset, None, osc_bg), fallback);

        // (e) Nothing → the fallback pair.
        assert_eq!(v6_default_pair(unset, None, None), fallback);

        // A named (non-Rgb) theme colour is not "supplied" — the pixel canvas needs
        // real bytes, so terminal_default White/Black falls through to the OSC pair.
        let named = Style::default().fg(Color::White).bg(Color::Black);
        assert_eq!(v6_default_pair(named, osc_fg, osc_bg), osc_pair);
    }

    #[test]
    fn content_bounds_never_clamps_a_layered_v6_root() {
        // The v6 raster/hybrid paths scale pixel content to the pane; clamping
        // to the cell content_size pinned the game to a native-size stamp in
        // the corner of a large terminal (the live "tiny render" bug).
        let model = hybrid_v6_model(); // Layered root, content_size (40, 25)
        let area = Rect::new(0, 0, 210, 55);
        assert_eq!(content_bounds(&model, area), area, "Layered root gets the full pane");
    }

    /// The live input line and its caret must NOT depend on which pane holds the
    /// keyboard. This was focus-gated on the Glulx/v6 raster path, so opening a room
    /// panel (or reaching the inspector via select-room) made the caret and your
    /// half-typed command disappear with no sign they were still buffered — while the
    /// Z-machine transcript path, which has no such gate, kept showing them.
    #[test]
    fn the_live_input_line_shows_regardless_of_which_pane_has_focus() {
        let cols = 40u16;
        let rows = 10u16;
        for focus in [crate::state::Focus::Game, crate::state::Focus::Map] {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.push_transcript("You are standing in an open field.");
            for ch in "open mailbox".chars() {
                state.input.value.push(ch);
            }
            state.input.cursor = state.input.value.chars().count();
            state.focus = focus;

            let (main, _) = build_main_text(&state, cols, rows);
            assert!(
                main.awaiting,
                "the input line must render with focus {focus:?} — it is not a focus indicator"
            );
            assert_eq!(main.input, "open mailbox", "the buffered command must be carried through");
            assert_eq!(main.cursor_col, "open mailbox".chars().count() as u16, "caret sits after the text");
        }
    }

    #[test]
    fn build_main_text_floats_inline_image_and_narrows_beside_it() {
        // A transcript-anchored inline image (32×64 → 4 rows at FONT_H 16, margin
        // 40px → 5 cols) becomes a float: it occupies no text row, the 4 rows
        // beside it wrap narrower, and rows past it wrap at full width.
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(32, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px: Some(40),
            source: crate::inline_image::ImageSource::Story,
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image line became a float, not a text row");
        let f = &main.floats[0];
        assert_eq!((f.row, f.rows, f.reserve_cols, f.text_col, f.img_col), (1, 4, 5, 5, 0), "anchored after 'before', 64px/16 = 4 rows, 40px/8 = 5 cols, left float");
        assert_eq!(main.lines[0], "before");
        // Rows 1..5 (beside the float) wrap at 40-5=35 cols; later rows full width.
        for (i, row) in main.lines.iter().enumerate().skip(1) {
            let w = row.chars().count();
            if (1..5).contains(&i) {
                assert!(w <= 35, "row {i} beside the float is narrow, got {w}");
            } else {
                assert!(w <= 40, "row {i} is full width, got {w}");
            }
        }
        assert!(main.lines[5..].iter().any(|r| r.chars().count() > 35), "rows past the float use full width");
    }

    #[test]
    fn build_main_text_bands_inline_up_content_art_full_width() {
        // Shogun's opening ship illustration is a window-0 picture classified
        // InlineUp by `win0_pic_align` (content-art sized, SQ-0471) — unlike a
        // MarginLeft drop-cap it must NOT float with text wrapped beside it: it
        // reserves full-width blank rows, text stops above and resumes below,
        // never over it (SQ-0470 follow-up).
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(160, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None,
            margin_px: None,
            source: crate::inline_image::ImageSource::Story,
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image still carries its pixels for the canvas blit");
        let f = &main.floats[0];
        // 64px / FONT_H(16) = 4 rows, anchored right after "before" (row 1).
        assert_eq!((f.row, f.rows), (1, 4), "band anchored after 'before', 64px/16 = 4 rows");
        assert_eq!(main.lines[0], "before");
        // Every row the band spans is blank — no text row overlaps its rows.
        for (i, row) in main.lines.iter().enumerate() {
            if (f.row as usize..f.row as usize + f.rows as usize).contains(&i) {
                assert!(row.is_empty(), "row {i} is inside the band and must carry no text, got {row:?}");
            }
        }
        // Text resumes below the band, at full (unindented) width — long enough
        // to prove it isn't narrowed the way a MarginLeft float would narrow it.
        assert!(!main.lines[5].is_empty(), "text resumes right after the band");
        assert!(main.lines[5].chars().count() > 35, "row 5 is full width, got {:?}", main.lines[5]);
    }

    #[test]
    fn build_main_text_right_float_narrows_left_and_places_picture_right() {
        // SQ-0489: Shogun's opening — a MarginRight window-0 picture floats at the
        // RIGHT edge; the prose rows beside it stay flush LEFT but narrow, and rows
        // past the picture reclaim full width. (160px → 20 cols; reserve 21; in a
        // 40-col box the text column is 19 cols.)
        let mut state = crate::state::AppState::default();
        state.push_transcript_kind("before", crate::state::TranscriptKind::Story);
        state.push_transcript_image(crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::from_pixel(160, 64, image::Rgba([9, 9, 9, 255]))),
            align: crate::inline_image::ImageAlign::MarginRight,
            scaled: None,
            margin_px: None,
            source: crate::inline_image::ImageSource::Story,
        });
        let para = "word ".repeat(40);
        state.push_transcript_kind(para.trim_end(), crate::state::TranscriptKind::Story);
        let (main, _) = build_main_text(&state, 40, 30);
        assert_eq!(main.floats.len(), 1, "the image became a right float");
        let f = &main.floats[0];
        // 64px/16 = 4 rows; 160px/8 = 20 img cols; reserve = 21; img_col = 40-20 = 20.
        assert_eq!((f.row, f.rows, f.reserve_cols, f.text_col, f.img_col), (1, 4, 21, 0, 20), "right float geometry");
        assert_eq!(main.lines[0], "before");
        // Rows 1..5 (beside the float) wrap at 40-21 = 19 cols; later rows widen.
        for (i, row) in main.lines.iter().enumerate().skip(1) {
            let w = row.chars().count();
            if (1..5).contains(&i) {
                assert!(w <= 19, "row {i} beside the right float is narrow, got {w}");
            }
        }
        assert!(main.lines[5..].iter().any(|r| r.chars().count() > 19), "rows past the float use full width");
    }

    #[test]
    fn build_main_text_maps_style_runs_onto_wrapped_rows() {
        // SQ-0540: the raster prose path carries per-char emphasis, so the
        // synthesized bold/italic faces land on the same characters the terminal
        // transcript emphasises. The mapping must survive word wrapping: a run's
        // char offsets index the UNWRAPPED line.
        let mut state = crate::state::AppState::default();
        // 3 words of 5 chars: "aaaaa bbbbb ccccc" wraps to 2 rows at 12 cols.
        state.push_transcript_kind("aaaaa bbbbb ccccc", crate::state::TranscriptKind::Story);
        state.transcript_runs.resize(state.transcript.len(), Vec::new());
        let last = state.transcript.len() - 1;
        state.transcript_runs[last] = vec![
            // "bbbbb" bold (chars 6..11), the trailing "cc" of "ccccc" italic.
            crate::state::StyleRun { start: 6, end: 11, bits: 2, fg: 0, bg: 0, link: 0, glk_style: 0 },
            crate::state::StyleRun { start: 15, end: 17, bits: 4, fg: 0, bg: 0, link: 0, glk_style: 0 },
        ];
        let (main, _) = build_main_text(&state, 12, 8);
        assert_eq!(main.lines, vec!["aaaaa bbbbb", "ccccc"], "wraps into two rows");
        assert_eq!(main.styles.len(), main.lines.len(), "styles stay parallel to lines");
        assert_eq!(main.styles[0], vec![0, 0, 0, 0, 0, 0, 2, 2, 2, 2, 2], "row 0: only 'bbbbb' is bold");
        assert_eq!(main.styles[1], vec![0, 0, 0, 4, 4], "row 1: the run is rebased past the dropped wrap space");

        // Reverse/fixed-pitch bits are dropped (no block to swap in the prose
        // raster, and the bitmap font is fixed-pitch already) — and a line with
        // no emphasis at all allocates no style row.
        state.transcript_runs[last] = vec![crate::state::StyleRun { start: 0, end: 17, bits: 1 | 8, fg: 0, bg: 0, link: 0, glk_style: 0 }];
        let (main, _) = build_main_text(&state, 12, 8);
        assert!(main.styles.iter().all(|r| r.is_empty()), "reverse/fixed-pitch leave every row roman, got {:?}", main.styles);
    }

    #[test]
    fn build_main_text_honors_transcript_scroll_offset() {
        // 20 short story lines into a 6-row story box (budget = 5 body rows). The
        // visible slice must window by `effective_transcript_scroll` (rows from the
        // bottom), clamped to `max_scroll`, newest-at-bottom when the offset is 0.
        let mut state = crate::state::AppState::default();
        for k in 0..20 {
            state.push_transcript_kind(&format!("L{k}"), crate::state::TranscriptKind::Story);
        }
        // Offset 0: the newest 5 rows (L15..=L19).
        state.transcript_scroll = 0;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(m.total_rows, 20);
        assert_eq!(m.viewport_rows, 5, "6 story-box rows minus the input line");
        assert_eq!(m.max_scroll, 15, "20 total - 5 body");
        assert_eq!(main.lines, vec!["L15", "L16", "L17", "L18", "L19"]);
        assert_eq!(m.first_visible_row, 15);

        // Scrolled back 3: the window shifts up by 3 (L12..=L16).
        state.transcript_scroll = 3;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(main.lines, vec!["L12", "L13", "L14", "L15", "L16"]);
        assert_eq!(m.first_visible_row, 12);

        // Over-scroll past the top clamps to max_scroll: the oldest 5 rows.
        state.transcript_scroll = 999;
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(main.lines, vec!["L0", "L1", "L2", "L3", "L4"]);
        assert_eq!(m.first_visible_row, 0);
    }

    #[test]
    fn build_main_text_short_transcript_shows_all_and_never_scrolls() {
        // Fewer wrapped rows than the budget: everything is visible, max_scroll is
        // 0, and any scroll offset is a no-op (the view stays pinned at the bottom).
        let mut state = crate::state::AppState::default();
        for k in 0..3 {
            state.push_transcript_kind(&format!("L{k}"), crate::state::TranscriptKind::Story);
        }
        state.transcript_scroll = 7; // clamped to 0
        let (main, m) = build_main_text(&state, 40, 6);
        assert_eq!(m.total_rows, 3);
        assert_eq!(m.max_scroll, 0, "content fits — nothing to scroll");
        assert_eq!(main.lines, vec!["L0", "L1", "L2"]);
        assert_eq!(m.first_visible_row, 0);
    }

    /// Build a `Theme` with the given selectors' bg overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to
    /// `theme.get("<selector>")` (SQ-0309) can still inject a custom colour
    /// instead of mutating the (no-longer-read) legacy `ColorScheme` field.
    fn theme_with_bg_overrides(overrides: &[(&str, ratatui::style::Color)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, bg) in overrides {
            decls.insert(sel.to_string(), crate::theme::registry::Delta { bg: Some(bg), ..crate::theme::registry::Delta::EMPTY });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    #[test]
    fn reserve_text_margin_insets_caps_and_noops_at_zero() {
        let mut state = crate::state::AppState::default();
        let fill = ratatui::style::Style::default();
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        let area = Rect::new(0, 0, 20, 10);

        // Zero margin returns the rect untouched.
        state.config.text_margin_x = 0;
        state.config.text_margin_y = 0;
        assert_eq!(reserve_text_margin(area, &state, fill, &mut buf), area);

        // (2,1) reserves 2 columns each side and 1 row top+bottom.
        state.config.text_margin_x = 2;
        state.config.text_margin_y = 1;
        assert_eq!(reserve_text_margin(area, &state, fill, &mut buf), Rect::new(2, 1, 16, 8));

        // An over-large margin is capped so at least one cell of text survives.
        state.config.text_margin_x = 100;
        state.config.text_margin_y = 100;
        let got = reserve_text_margin(area, &state, fill, &mut buf);
        assert!(got.width >= 1 && got.height >= 1, "capped margin keeps >=1 cell: {got:?}");
    }

    #[test]
    fn simple_path_transcript_geometry_is_inset_by_text_margin() {
        // The rect render_transcript publishes as `transcript_geom` (what mouse
        // selection maps through) must shrink by exactly the configured margin, so
        // the inset stays consistent with clicks and the copy path (SQ-0345).
        let published = |mx: u16, my: u16| {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.config.text_margin_x = mx;
            state.config.text_margin_y = my;
            for k in 0..5 { state.push_transcript(&format!("line {k}")); }
            let model = ScreenModel {
                root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
                status: StatusModel::HostManaged,
                bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
                fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
                content_size: (0, 0),
            };
            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            state.transcript_geom.get().expect("transcript geom published").area
        };
        let base = published(0, 0);
        let inset = published(3, 2);
        assert_eq!(inset.x, base.x + 3, "left margin reserved");
        assert_eq!(inset.y, base.y + 2, "top margin reserved");
        assert_eq!(inset.width, base.width - 6, "both horizontal margins reserved");
        assert_eq!(inset.height, base.height - 4, "top+bottom margins reserved");
    }

    /// SQ-0532/A-F1. ZMSD §8.4: the interpreter "may change the exact dimensions
    /// whenever it likes but must write the current height (in lines) and width
    /// (in characters) into bytes $20 and $21 in the header." What we report is
    /// therefore MEASURED from the story pane, not a fixed 80x24 guess.
    #[test]
    fn story_screen_dims_measure_the_story_pane() {
        let state = frameless_state(); // upper-window frame themed off
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((30, 99)),
            "a bare pane reports its own cell size, less the scrollbar gutter column"
        );
        assert_eq!(
            story_screen_dims(Rect::new(4, 2, 62, 17), &state),
            Some((17, 61)),
            "the pane's position is irrelevant; only its extent is reported"
        );
        // A hidden or not-yet-measured pane has nothing to report.
        assert_eq!(story_screen_dims(Rect::new(0, 0, 0, 0), &state), None);
        assert_eq!(story_screen_dims(Rect::new(0, 0, 80, 0), &state), None);
    }

    #[test]
    fn story_screen_dims_subtract_the_margin_and_the_upper_window_frame() {
        // The declared screen is the region the game's own screen actually gets:
        // the text margin is where the transcript wraps, and the upper window's
        // frame is drawn AROUND the grid, so both come off the reported size.
        let mut state = frameless_state();
        state.config.text_margin_x = 3;
        state.config.text_margin_y = 2;
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((30, 93)),
            "horizontal margin comes off the width; the grid is never inset vertically"
        );
        state.config.text_margin_x = 0;
        state.colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::Single);
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 100, 30), &state),
            Some((28, 97)),
            "a framed upper window loses one row/column per drawn side"
        );
    }

    #[test]
    fn story_screen_dims_honour_a_pinned_config_override() {
        // `virtual_screen_cols`/`rows` stay available for pinning a fixed virtual
        // screen; an unset key follows the pane (see the config docs).
        let mut state = frameless_state();
        state.config.virtual_screen_cols = Some(80);
        assert_eq!(
            story_screen_dims(Rect::new(0, 0, 132, 40), &state),
            Some((40, 80)),
            "a pinned width wins; the unset height still follows the pane"
        );
        state.config.virtual_screen_rows = Some(24);
        assert_eq!(story_screen_dims(Rect::new(0, 0, 132, 40), &state), Some((24, 80)));
    }

    /// SQ-0679: the width DECLARED to a v4+ story never drops below the width
    /// it booted at, because a v4/v5 status routine reads $21 once and bakes
    /// its field columns in — narrow the screen under it and those columns
    /// fall outside the window, where §8.7.2.3 makes the `set_cursor` illegal and
    /// the digits land on the room name instead. Widening still follows the pane
    /// (SQ-0533), and the HEIGHT always does: `split_window` re-declares it on
    /// every layout.
    ///
    /// SQ-0680: the floor is `boot_cols`, THIS session's actual boot width —
    /// `zvm::screen::DEFAULT_SCREEN_COLS` (80) unseeded, matching the original
    /// SQ-0679 assumption, or whatever narrower/wider pane the caller pre-boot
    /// seeded (`GameSession::boot_screen_cols`).
    #[test]
    fn declared_width_never_drops_below_the_boot_width() {
        let state = frameless_state();
        let narrow = Rect::new(0, 0, 60, 20);
        let wide = Rect::new(0, 0, 132, 40);
        let boot_80 = zvm::screen::DEFAULT_SCREEN_COLS as u16;
        // The raw pane measurement is unchanged — it still measures the pane.
        assert_eq!(story_screen_dims(narrow, &state), Some((20, 59)));

        // v5: floored at the boot width going down, free to follow the pane up.
        assert_eq!(
            declared_story_screen_dims(narrow, &state, 5, boot_80),
            Some((20, 80)),
            "a 59-column pane still declares the 80 columns the story booted with"
        );
        assert_eq!(
            declared_story_screen_dims(wide, &state, 5, boot_80),
            Some((40, 131)),
            "a wider pane is declared in full — every old coordinate is still inside it"
        );
        // The height follows the pane in both directions.
        assert_eq!(declared_story_screen_dims(narrow, &state, 5, boot_80).unwrap().0, 20);

        // v3 has no such header fields, and v6's screen is its native pixel
        // frame — neither is floored.
        assert_eq!(declared_story_screen_dims(narrow, &state, 3, boot_80), Some((20, 59)));
        assert_eq!(declared_story_screen_dims(narrow, &state, 6, boot_80), Some((20, 59)));

        // An explicitly pinned width is the user's, not ours to floor.
        let mut pinned = frameless_state();
        pinned.config.virtual_screen_cols = Some(40);
        assert_eq!(declared_story_screen_dims(narrow, &pinned, 5, boot_80), Some((20, 40)));

        // SQ-0680: a session pre-boot-seeded to a NARROWER pane floors at ITS
        // own boot width, not the fixed 80 default — a 60-column pane that
        // booted at 60 must not be forced back up to 80 on the next poll,
        // which would silently undo the whole point of seeding it.
        assert_eq!(
            declared_story_screen_dims(narrow, &state, 5, 60),
            Some((20, 60)),
            "a 59-column pane under a 60-column boot floors at the boot width, not 80"
        );
        // …and a pane exactly at (or wider than) that boot width is reported
        // as-measured, same as always.
        assert_eq!(declared_story_screen_dims(wide, &state, 5, 60), Some((40, 131)));
    }

    /// SQ-0532/A-F1(c): the width the story is TOLD about, the width the upper
    /// window is RENDERED at, and the width the transcript WRAPS at are one
    /// number. Before this, the grid was sized from a fixed 80-column header and
    /// centred in the pane while the prose wrapped at the pane's real width, so a
    /// game's full-width form sat offset from the text beside it.
    #[test]
    fn declared_width_equals_rendered_grid_width_equals_transcript_wrap() {
        let state = frameless_state(); // no upper-window frame, no text margin
        let area = Rect::new(0, 0, 60, 12);
        let (_, cols) = story_screen_dims(area, &state).expect("pane measured");
        assert_eq!(cols, area.width - 1, "with no frame and no margin, the pane less its scrollbar gutter");

        // A grid sized the way `split_window` sizes it — from header byte $21.
        let mut grid = crate::engine::GridWindow { active_rows: 1, ..Default::default() };
        grid.resize(1, cols);
        grid.put(1, 1, '<', 0);
        grid.put(1, cols, '>', 0);
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let used = draw_upper_window(&grid, false, &state.colors, area, &mut buf, true, &mut links);
        assert_eq!(used, 1, "one grid row, no frame rows");
        // Rendered edge-to-edge across the pane: no centring offset left to drift.
        assert_eq!(buf.cell((area.x, area.y)).unwrap().symbol(), "<");
        assert_eq!(buf.cell((area.x + cols - 1, area.y)).unwrap().symbol(), ">");

        // The transcript below wraps at that same width (its rightmost column is
        // the scrollbar gutter, which is chrome, not story columns).
        let mut state2 = frameless_state();
        state2.push_transcript("x");
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height - used);
        let (_, _, _, _) = render_transcript(&model.status, None, &state2, tarea, &mut buf, None);
        let geom = state2.transcript_geom.get().expect("geometry published").area;
        assert_eq!(geom.width, cols, "transcript wraps at the declared width");
        assert_eq!(geom.x, area.x, "and starts at the same column the grid does");
    }

    #[test]
    fn scrollbar_sits_at_border_not_inside_text_margin() {
        // With a horizontal text margin, only the text is inset — the scrollbar
        // must stay flush against the pane border (rightmost column), never inside
        // the margin band (SQ-0345).
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let mx = 3;
        state.config.text_margin_x = mx;
        // Far more lines than the viewport → scrollbar must appear.
        for k in 0..80 { state.push_transcript(&format!("line {k}")); }
        let model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::Default),
            content_size: (0, 0),
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        let column: String = (0..area.height)
            .map(|y| buf.cell((area.width - 1, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(column.contains('█'), "scrollbar thumb should render in the border column, not inset by the margin: {column:?}");
        // And the inset column (where the scrollbar used to sit) must be clear of it.
        let inset_col: String = (0..area.height)
            .map(|y| buf.cell((area.width - 1 - mx, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!inset_col.contains('█'), "no scrollbar inside the margin band: {inset_col:?}");
    }

    #[test]
    fn garglk_margin_overrides_config_default() {
        // A discovered garglk.ini's tmargin wins over the global config margin
        // (SQ-0344, highest precedence).
        let mut state = crate::state::AppState::default();
        state.config.text_margin_x = 0;
        state.config.text_margin_y = 0;
        state.garglk_overlay = Some(crate::garglk_ini::GarglkOverlay {
            margin_x: Some(3),
            margin_y: Some(1),
            ..Default::default()
        });
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        let got = reserve_text_margin(Rect::new(0, 0, 20, 10), &state, ratatui::style::Style::default(), &mut buf);
        assert_eq!(got, Rect::new(3, 1, 14, 8), "garglk tmargin applied over the zero config default");
    }

    fn grid_with(text: &str) -> GridWindow {
        let mut g = GridWindow::default();
        g.resize(1, text.chars().count() as u16);
        for (i, ch) in text.chars().enumerate() {
            g.put(1, i as u16 + 1, ch, 0);
        }
        g.active_rows = 1;
        g
    }

    fn model_with_page(bg: zvm::screen::ZColour, fg: zvm::screen::ZColour) -> ScreenModel {
        ScreenModel {
            root: WinNode::Blank,
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(bg),
            fg: crate::state::pack_zcolour(fg),
            content_size: (0, 0),
        }
    }

    #[test]
    fn grid_scheme_overrides_upper_window_with_game_page_colours() {
        // A game that set a black-on-white page (CounterfeitMonkey) → the grid base
        // becomes that page, so a reverse-video status line reverses to white-on-black
        // instead of reversing the app theme. (SQ-0262)
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Owned(_)), "override clone when the game set a scheme");
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)));
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)));
    }

    #[test]
    fn grid_scheme_also_paints_the_border_in_the_game_page_colours() {
        // SQ-0267: the status border is our own chrome (Glk sends no border style),
        // so it must adopt the game's page colours too — the whole status block
        // (content + frame) reads as one coloured unit on the recoloured page.
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));
        let gc = grid_scheme(&state, &model);
        assert_eq!(gc.theme.get("upper_window_border").style.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)),
            "border background matches the game page background");
        assert_eq!(gc.theme.get("upper_window_border").style.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)),
            "border line drawn in the game page foreground ink");
    }

    /// End-to-end guard for the same fix: render the simple (Z-machine) path with
    /// a game-set page scheme and check the actually-painted grid/border pixels,
    /// not just `grid_scheme`'s returned struct.
    #[test]
    fn simple_path_grid_and_border_paint_the_game_page_colours() {
        use ratatui::style::Color;
        let mut grid = grid_with("HI");
        grid.border = BorderPref::Border;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x00FF_FFFF)),
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0)),
            content_size: (0, 0),
        };
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // uw_w = 2 + 2 borders = 4, centered in 20 → x_off = 8; frame corner at
        // (8,0), content at (9,1) (mirrors `simple_path_still_frames_a_bordered_grid`).
        let border_cell = buf.cell((8, 0)).unwrap().style();
        assert_eq!(border_cell.fg, Some(Color::Rgb(0, 0, 0)), "border painted in the game page fg");
        assert_eq!(border_cell.bg, Some(Color::Rgb(255, 255, 255)), "border painted in the game page bg");
        let content_cell = buf.cell((9, 1)).unwrap().style();
        assert_eq!(content_cell.bg, Some(Color::Rgb(255, 255, 255)), "grid content painted in the game page bg");
    }

    // ── SQ-0510: the probe-seeded chrome vs. the game's own page ─────────────

    /// A `ColorScheme` whose theme was built the way `reload_style` builds it on
    /// a terminal that answered the OSC 10/11 probe with no scheme configured:
    /// chrome follows the terminal's real page instead of the hard-coded black.
    fn probe_seeded_colors() -> crate::colors::ColorScheme {
        let probe = crate::term_colors::TermDefaultColors {
            fg: Some(image::Rgba([0x58, 0x6e, 0x75, 255])),
            bg: Some(image::Rgba([0xfd, 0xf6, 0xe3, 255])),
        };
        let gs = crate::colors::seed_scheme_from_terminal(
            crate::colors::GhosttyScheme::default(),
            &probe,
        );
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.theme = crate::theme::resolve::resolve_theme(
            &gs,
            &crate::theme::toml_schema::ParsedStyle::default(),
        );
        cs
    }

    #[test]
    fn a_game_page_still_beats_the_probe_seeded_chrome() {
        // SQ-0262 must survive SQ-0510: seeding chrome from the terminal fixes the
        // case where NOBODY set a colour; a game that DOES set its page still owns
        // the grid. Run both `honor_game_colours` modes — with the gate on the game
        // wins, with it off the seeded terminal page stands (and neither is black).
        use ratatui::style::Color;
        use zvm::screen::ZColour;
        let model = model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0));

        let mut state = AppState::default();
        state.colors = probe_seeded_colors();
        assert_eq!(
            state.colors.theme.get("upper_window").style.bg,
            Some(Color::Rgb(0xfd, 0xf6, 0xe3)),
            "precondition: the seeded chrome is the probed terminal page"
        );

        // honor = true (the shipped default): the game's white page wins outright.
        state.config.honor_game_colours = true;
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Owned(_)), "a game page still forces the override clone");
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(Color::Rgb(0, 0, 0)));
        assert_eq!(gc.theme.get("upper_window_border").style.bg, Some(Color::Rgb(255, 255, 255)));

        // honor = false: the game is ignored and the seeded terminal page stands —
        // still not the old hard-coded black.
        state.config.honor_game_colours = false;
        let gc = grid_scheme(&state, &model);
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)));
        assert_eq!(gc.theme.get("upper_window").style.bg, Some(Color::Rgb(0xfd, 0xf6, 0xe3)));
    }

    #[test]
    fn a_game_that_sets_only_ink_keeps_the_probe_seeded_page() {
        // The half-set case `grid_scheme` has always handled: the game names an
        // ink but no page, so the page comes from the theme's chrome. That used to
        // mean black; with the probe answered it is the terminal's own page.
        use ratatui::style::Color;
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = probe_seeded_colors();
        state.config.honor_game_colours = true;
        let model = model_with_page(ZColour::Default, ZColour::True24(0x00FF_0000));

        let gc = grid_scheme(&state, &model);
        assert_eq!(gc.theme.get("upper_window").style.fg, Some(Color::Rgb(255, 0, 0)), "the game's ink");
        assert_eq!(
            gc.theme.get("upper_window").style.bg,
            Some(Color::Rgb(0xfd, 0xf6, 0xe3)),
            "and the terminal's page beneath it"
        );
    }

    #[test]
    fn grid_scheme_borrows_theme_when_game_set_no_page() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = true;
        let gc = grid_scheme(&state, &model_with_page(ZColour::Default, ZColour::Default));
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)), "theme unchanged when no game page colours");
    }

    #[test]
    fn grid_scheme_borrows_theme_when_colours_disabled() {
        use zvm::screen::ZColour;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.config.honor_game_colours = false;
        let gc = grid_scheme(&state, &model_with_page(ZColour::True24(0x00FF_FFFF), ZColour::True24(0)));
        assert!(matches!(gc, std::borrow::Cow::Borrowed(_)), "game colours off → theme borrowed, override inert");
    }

    fn inline_buffer(line: &str) -> BufferWindow {
        BufferWindow {
            lines: vec![line.to_string()],
            runs: vec![Vec::new()],
            para: vec![crate::state::ParaFmt::default()],
            images: vec![None],
            scroll: 0,
            primary: false,
            bg: None,
            fg: None,
            panel: false,
            px_runs: Vec::new(),
        }
    }

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
    }

    /// SQ-0332: a deep, Kerkerkruip-shaped multi-window tree (nested bordered
    /// pairs, side panels + a main window + graphics-rule separators) renders with
    /// EVERY text pane painted in its own Normal-style background at its exact rect.
    /// Reconstructed from a live `/dump-windows` (165×60). Guards the render math
    /// (leaves must land on their dumped coordinates) and the per-window fills —
    /// the visible corruption came from a STALE tree (`bg = None`), not this path.
    #[test]
    fn deep_multiwindow_tree_paints_every_pane() {
        use ratatui::style::Color;
        fn gfx() -> WinNode {
            let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
            WinNode::Graphics(crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false })
        }
        fn buf(bg: u32, primary: bool) -> WinNode {
            WinNode::Buffer(BufferWindow { lines: vec![], runs: vec![], para: vec![], images: vec![], scroll: 0, primary, bg: Some(bg), fg: None, panel: false, px_runs: Vec::new() })
        }
        fn grid(bg: u32) -> WinNode {
            let mut g = GridWindow::default();
            g.resize(1, 1);
            g.active_rows = 1;
            g.bg = Some(bg);
            WinNode::Grid(g)
        }
        fn pair(vertical: bool, split: u16, first: WinNode, second: WinNode) -> WinNode {
            WinNode::Pair { vertical, split: Split { fixed: split }, border: true, key_bg: None, key_fg: None, first: Box::new(first), second: Box::new(second) }
        }
        let root =
            pair(false, 123,
                pair(false, 121,
                    pair(false, 36,
                        pair(false, 1, gfx(),
                            pair(false, 32,
                                pair(true, 58,
                                    pair(true, 1, buf(0xDDDDDD, false),        // buf75 header @(2,0)
                                        pair(true, 1, gfx(), buf(0xEEEEEE, false))), // buf67 body @(2,4)
                                    gfx()),
                                gfx())),
                        pair(true, 1, grid(0xDDDDDD),                          // grid79 @(37,0)
                            pair(true, 1, gfx(), buf(0xFFFFFF, true)))),        // buf4 main @(37,4)
                    gfx()),
                pair(false, 39,
                    pair(true, 1, buf(0xDDDDDD, false),                        // buf53 @(124,0)
                        pair(true, 1, gfx(),
                            pair(true, 42,
                                pair(true, 40, buf(0xEEEEEE, false), gfx()),   // buf47 @(124,4)
                                pair(true, 11,
                                    pair(true, 1, buf(0xDDDDDD, false),        // buf63 @(124,47)
                                        pair(true, 1, gfx(), buf(0xEEEEEE, false))), // buf57 @(124,51)
                                    gfx())))),
                    gfx()));
        let model = ScreenModel {
            root,
            status: StatusModel::HostManaged,
            bg: crate::state::pack_zcolour(zvm::screen::ZColour::True24(0xFFFFFF)),
            fg: 0,
            content_size: (165, 60),
        };
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.theme = theme_with_bg_overrides(&[
            ("transcript", Color::Rgb(9, 9, 9)), // sentinel: an unpainted pane shows this
            ("upper_window", Color::Rgb(9, 9, 9)),
        ]);
        let mut state = AppState::default();
        state.colors = colors;
        state.config.honor_game_colours = true;
        let area = Rect::new(0, 0, 165, 60);
        let mut b = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut b);

        let bgc = |x: u16, y: u16| b.cell((x, y)).unwrap().style().bg;
        // Each pane paints its own Normal bg at its dumped rect (not the sentinel).
        assert_eq!(bgc(3, 10), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "left panel body buf67 @(2,4)");
        assert_eq!(bgc(3, 0), Some(Color::Rgb(0xDD, 0xDD, 0xDD)), "left panel header buf75 @(2,0)");
        assert_eq!(bgc(60, 20), Some(Color::Rgb(0xFF, 0xFF, 0xFF)), "main window buf4 @(37,4)");
        assert_eq!(bgc(130, 20), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "right panel buf47 @(124,4)");
        assert_eq!(bgc(130, 53), Some(Color::Rgb(0xEE, 0xEE, 0xEE)), "lower-right buf57 @(124,51)");
        assert_eq!(bgc(130, 47), Some(Color::Rgb(0xDD, 0xDD, 0xDD)), "lower-right header buf63 @(124,47)");
        // No text pane left showing the sentinel (every pane painted).
        assert_ne!(bgc(3, 10), Some(Color::Rgb(9, 9, 9)));
    }

    /// SQ-0325 follow-up: the between-siblings separator is drawn in the split's
    /// KEY window colour — `key_fg` on `key_bg` — rather than the plain theme
    /// border style. Each channel falls back to the theme when `None`.
    #[test]
    fn separator_adopts_key_window_colour() {
        use ratatui::style::Color;
        let colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 5, 1);
        let mut buf = Buffer::empty(area);
        // Vertical pair → horizontal rule; key fg red (0xFF0000), key bg blue (0x0000FF).
        draw_window_separator(area, true, Some(0x00FF_0000), Some(0x0000_00FF), &colors, &mut buf);
        let c = buf.cell((2, 0)).unwrap();
        assert_eq!(c.style().fg, Some(Color::Rgb(0xFF, 0, 0)), "rule fg is the key window fg");
        assert_eq!(c.style().bg, Some(Color::Rgb(0, 0, 0xFF)), "rule bg is the key window bg");
        assert_eq!(c.symbol(), "\u{2500}", "vertical pair draws a horizontal rule glyph");
    }

    /// The Scott split: a `panel: true` buffer over the primary transcript draws
    /// with the themed `room_panel` colour, distinct from the transcript colour, so
    /// the top and bottom regions read apart.
    #[test]
    fn room_panel_draws_with_room_panel_theme() {
        use ratatui::style::Color;
        let mut panel = inline_buffer("I'm in a forest");
        panel.panel = true;
        let root = WinNode::Pair {
            vertical: true,
            split: Split { fixed: 1 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(WinNode::Buffer(panel)),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        };
        let model = ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) };

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.theme = theme_with_bg_overrides(&[
            ("transcript", Color::Rgb(9, 9, 9)),
            ("room_panel", Color::Rgb(0, 0, 128)),
        ]);
        let mut state = AppState::default();
        state.colors = colors;
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Top row (panel) uses room_panel bg, distinct from the transcript region
        // below it.
        let bgc = |x: u16, y: u16| buf.cell((x, y)).unwrap().style().bg;
        assert_eq!(bgc(0, 0), Some(Color::Rgb(0, 0, 128)), "panel uses room_panel bg");
        assert_ne!(bgc(0, 0), bgc(0, 3), "panel and transcript regions read apart");
    }

    #[test]
    fn is_simple_classifies_trees() {
        // Z-machine shape: grid over a (non-primary) buffer.
        let zm = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(GridWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&zm));
        // Lone buffer: simple.
        let lone = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&lone));
        // Two buffers: not simple.
        let two = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 10 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(BufferWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&two));
    }

    /// SQ-0325: a grid split BESIDE the buffer (winmethod_Left/Right, a horizontal
    /// pair) must NOT be the simple path. The simple path always draws the grid as a
    /// full-width top status band over the transcript, so a side-by-side grid would
    /// be mis-rendered as a centered top bar with the buffer full-width below
    /// ("the window is centered and we lose the main window"). It must take the
    /// generic path, which honours the left/right geometry.
    #[test]
    fn grid_beside_buffer_is_not_simple() {
        let side = ScreenModel {
            root: WinNode::Pair {
                vertical: false, // horizontal pair = Left/Right split
                split: Split { fixed: 20 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(GridWindow::default())),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&side), "a grid beside the buffer must use the generic path");
    }

    /// SQ-0325: a grid split BELOW the buffer (winmethod_Below → buffer-above-grid,
    /// a vertical pair with the buffer first) is likewise not the simple shape —
    /// the simple path would still draw the grid on TOP, in the wrong place.
    #[test]
    fn buffer_above_grid_is_not_simple() {
        let below = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 22 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
                second: Box::new(WinNode::Grid(GridWindow::default())),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&below), "a grid below the buffer must use the generic path");
    }

    /// SQ-0325 end-to-end: a text grid opened to the LEFT of the main buffer renders
    /// as a full-height left column (its cells filling that column from the top-left),
    /// NOT centered on the top row. Regression guard for the mis-routing.
    #[test]
    fn left_grid_renders_in_left_column_not_top_bar() {
        // A 6-col grid whose row 0 reads "GRID" (filling from the left), split to the
        // left of the primary buffer at a 6-col boundary in a 20-wide pane.
        let mut grid = GridWindow::default();
        grid.resize(4, 6); // 4 rows, 6 cols — a full window, not a 1-row status line
        for (i, ch) in "GRID".chars().enumerate() {
            grid.put(1, i as u16 + 1, ch, 0);
        }
        grid.active_rows = 4;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 6 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // "GRID" fills the left column from column 0 on row 0 — not centered, not a
        // top status bar over a full-width transcript.
        assert_eq!(row_text(&buf, 0, 6), "GRID  ", "grid fills the left column: {:?}", row_text(&buf, 0, 6));
    }

    #[test]
    fn split_area_bordered_vertical_and_horizontal() {
        let area = Rect::new(0, 0, 20, 10);
        // Borderless (b=0): the gutter is empty, children abut.
        let (top, sep, bottom) = split_area_bordered(area, true, 3, 0);
        assert_eq!(top, Rect::new(0, 0, 20, 3));
        assert_eq!(sep, Rect::new(0, 3, 20, 0));
        assert_eq!(bottom, Rect::new(0, 3, 20, 7));
        let (left, sep, right) = split_area_bordered(area, false, 8, 0);
        assert_eq!(left, Rect::new(0, 0, 8, 10));
        assert_eq!(sep, Rect::new(8, 0, 0, 10));
        assert_eq!(right, Rect::new(8, 0, 12, 10));
        // Bordered (b=1): a 1-cell gutter is carved out between the children.
        let (top, sep, bottom) = split_area_bordered(area, true, 3, 1);
        assert_eq!(top, Rect::new(0, 0, 20, 3));
        assert_eq!(sep, Rect::new(0, 3, 20, 1));
        assert_eq!(bottom, Rect::new(0, 4, 20, 6));
        let (left, sep, right) = split_area_bordered(area, false, 8, 1);
        assert_eq!(left, Rect::new(0, 0, 8, 10));
        assert_eq!(sep, Rect::new(8, 0, 1, 10));
        assert_eq!(right, Rect::new(9, 0, 11, 10));
        // Oversized fixed clamps to the extent; the border can't overflow either.
        let (l2, sep, r2) = split_area_bordered(area, true, 99, 1);
        assert_eq!(l2.height, 10);
        assert_eq!(sep.height, 0);
        assert_eq!(r2.height, 0);
    }

    #[test]
    fn generic_renders_grid_and_two_inline_buffers_in_subrects() {
        // Grid (top row) over a left|right buffer split.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                // Grid border Unspecified + theme sides off → frameless (SQ-0286);
                // this test checks buffer subrects, not border chrome.
                first: Box::new(WinNode::Grid(grid_with("STATUS"))),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 10 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Buffer(inline_buffer("LEFT"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("RIGHT"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Grid "STATUS" drawn on the top row (centered in its 20-wide area).
        assert!(row_text(&buf, 0, 20).contains("STATUS"), "grid row: {:?}", row_text(&buf, 0, 20));
        // Row 1: LEFT buffer in cols [0,10), RIGHT buffer in cols [10,20).
        assert_eq!(row_text(&buf, 1, 4), "LEFT");
        let right = row_text(&buf, 1, 20);
        assert!(right[10..].contains("RIGHT"), "right buffer at col>=10: {:?}", right);
    }

    /// SQ-0303 Stage 2: in the game-managed multi-window (generic) path the app
    /// must NOT frame the grid or borrow rows — the game owns the layout and draws
    /// its own borders (Kerkerkruip renders its panel rules as graphics windows).
    /// The grid renders frameless at its exact 1-row rect, the buffer below starts
    /// at the grid's exact bottom (not +2), and the columns stay row-aligned — even
    /// when the grid carries an explicit `winmethod_Border` and the theme has every
    /// border side on. (Replaces the old SQ-0200 border-row-borrow behavior.)
    #[test]
    fn generic_grid_renders_frameless_without_borrowing_rows() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        // Kerkerkruip-shaped: a center column of an explicit-Border status grid
        // over an inline BODY buffer, beside a right column of a graphics rule
        // (the game's own separator) + an inline SIDE panel. The graphics leaf
        // forces the generic path.
        let mut grid = grid_with("ST");
        grid.border = BorderPref::Border; // explicit winmethod_Border
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 8 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Pair {
                    vertical: true,
                    split: Split { fixed: 1 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Grid(grid)),
                    second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
                }),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 1 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(graphics_node()),
                    second: Box::new(WinNode::Buffer(inline_buffer("SIDE"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(!is_simple(&model));

        // Theme with EVERY border side on — the old code would have framed the grid
        // and borrowed 2 rows; the fix suppresses both on the generic path.
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::Single;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::Single);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // Frameless: NO box-drawing glyph in the grid/body columns [0,8). (Col 8 is
        // the game's own graphics rule, which legitimately renders as a │ — SQ-0332.)
        for y in 0..10 {
            for x in 0..8 {
                let s = buf.cell((x, y)).unwrap().symbol();
                assert!(
                    !"┌┐└┘─│".contains(s),
                    "no frame glyph on the generic path, found {s:?} at ({x},{y})"
                );
            }
        }
        // The graphics rule column (col 8) DOES draw a thin │ rule (the game's divider).
        assert_eq!(buf.cell((8, 5)).unwrap().symbol(), "\u{2502}", "graphics window renders its own thin rule");
        // Grid "ST" sits frameless on row 0 (cols=2 centered in the 8-wide center
        // column: x_off=(8-2)/2=3).
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "S", "grid content on row 0, no top border");
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), "T");
        // No row borrowed: the BODY buffer starts at the grid's EXACT bottom (row 1),
        // not shoved to row 3 by a 2-row border-borrow.
        assert_eq!(row_text(&buf, 1, 4), "BODY", "buffer below starts at grid bottom (row 1), not +2");
        // Columns stay row-aligned: the SIDE panel's first line is on row 0, level
        // with the grid — the center column is not shifted down relative to it.
        let side = row_text(&buf, 0, 20);
        let side_tail: String = side.chars().skip(9).collect();
        assert!(side_tail.contains("SIDE"), "side panel level with grid on row 0: {side:?}");
    }

    /// SQ-0303 Stage 2 guard: the SIMPLE (Z-machine / lone-grid) path is unchanged
    /// — a `BorderPref::Border` grid over the primary buffer still draws its frame
    /// (via `draw_upper_window`), so Counterfeit Monkey's coloured status border
    /// (SQ-0267) is preserved. Only the generic path went frameless.
    #[test]
    fn simple_path_still_frames_a_bordered_grid() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let mut grid = grid_with("HI");
        grid.border = BorderPref::Border;
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid)),
                second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        assert!(is_simple(&model), "grid-over-primary-buffer is the simple path");

        // Theme sides OFF: BorderPref::Border still forces a fallback single frame.
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 20, 10);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // uw_w = 2 + 2 borders = 4, centered in 20 → x_off = 8; top-left corner at
        // (8,0), content pushed inside the frame to row 1.
        assert_eq!(buf.cell((8, 0)).unwrap().symbol(), "┌", "simple path still frames a Border grid");
        assert_eq!(buf.cell((9, 1)).unwrap().symbol(), "H", "content sits inside the frame");
    }

    /// SQ-0303: gvm snaps its working width down and leaves a blank margin, so the
    /// composite must clamp to `content_size` — the right-edge leaf keeps its own
    /// width instead of ballooning into the surplus, and the margin stays blank.
    #[test]
    fn generic_clamps_composite_to_content_size_leaving_margin_blank() {
        // Grid (top row) over a left|right buffer split, content 8 wide inside a
        // 12-wide render area → a 4-col snap-margin. Without the clamp the RIGHT
        // buffer (last right-spine leaf) would stretch to absorb cols 5..12.
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: false,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("ST"))),
                second: Box::new(WinNode::Pair {
                    vertical: false,
                    split: Split { fixed: 4 },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Buffer(inline_buffer("LEFT"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("RGHT"))),
                }),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (8, 6),
        };
        assert!(!is_simple(&model));

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;

        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The RIGHT buffer draws in the content box's right half [4,8), NOT stretched
        // to the pane's right edge.
        let row1 = row_text(&buf, 1, 12);
        assert!(row1[4..8].contains("RGHT"), "right buffer sits in cols 4..8: {:?}", row1);
        // The snap-margin (cols 8..12) is blank — no leaf stretched into it.
        assert_eq!(&row1[8..12], "    ", "snap-margin blank on row 1: {:?}", row1);
        // The margin is blank on every row (right strip is full-height).
        for y in 0..6 {
            let r = row_text(&buf, y, 12);
            assert_eq!(&r[8..12], "    ", "snap-margin blank on row {}: {:?}", y, r);
        }
    }

    #[test]
    fn inline_buffer_renders_styled_runs() {
        let mut b = inline_buffer("abCD");
        b.runs = vec![vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 10, 3);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 4), "abCD");
        // 'C' (col 2) carries the bold modifier.
        assert!(buf.cell((2, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(!buf.cell((0, 0)).unwrap().modifier.contains(ratatui::style::Modifier::BOLD));
    }

    #[test]
    fn inline_buffer_pushes_text_below_image_band() {
        // lines a / <image> / b. With a picker present the image at index 1
        // expands into a multi-row band, pushing "b" below the row it occupies
        // when images are off. Halfblocks font is 10x20 px; a 16x48-px image at
        // width 10 fits to a 2x3-cell band, so "b" lands on row 1 + 3 = 4.
        let mut px = image::RgbaImage::new(16, 48);
        for p in px.pixels_mut() {
            *p = image::Rgba([200, 40, 60, 255]);
        }
        let dummy = crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(px),
            align: crate::inline_image::ImageAlign::InlineUp,
            scaled: None, margin_px: None, source: crate::inline_image::ImageSource::Story,
        };
        let b = BufferWindow {
            lines: vec!["a".to_string(), String::new(), "b".to_string()],
            runs: vec![Vec::new(), Vec::new(), Vec::new()],
            para: vec![crate::state::ParaFmt::default(); 3],
            images: vec![None, Some(dummy), None],
            scroll: 0,
            primary: false,
            bg: None,
            fg: None,
            panel: false,
            px_runs: Vec::new(),
        };
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        let area = Rect::new(0, 0, 10, 8);
        let mut buf = Buffer::empty(area);
        render_inline_buffer(&b, &state, area, &mut buf);
        assert_eq!(row_text(&buf, 0, 1), "a", "first text line stays on row 0");
        let b_row = (0..8).find(|&y| row_text(&buf, y, 1).starts_with('b'));
        assert_eq!(b_row, Some(4), "\"b\" pushed below the 3-row image band");
    }

    #[test]
    fn story_pane_fills_game_background() {
        use ratatui::style::Color;
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // honor_game_colours defaults to true.
        let mut model = ScreenModel {
            root: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (0, 0),
        };
        model.bg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(2)); // black
        let area = Rect::new(0, 0, 10, 5);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);
        // A blank interior cell (the empty transcript body, not the bottom input
        // row) carries the game background (black). Retargeted for SQ-0532/A-F5:
        // the default palette now resolves Standard colours to their ZMSD §8.3.1
        // true-colour equivalents ("2 = black (true $0000)") rather than the named
        // ANSI colour, so black is the exact RGB (0,0,0).
        assert_eq!(buf.cell((0, 2)).unwrap().style().bg, Some(Color::Rgb(0, 0, 0)),
            "story pane blank cell painted with game background");
    }

    /// The Z-machine 2-node tree must render byte-identical through
    /// `render_story_pane` vs. the direct `draw_upper_window` + `render_transcript`
    /// path it replaces.
    #[test]
    fn zmachine_two_node_tree_is_byte_identical() {
        use zvm::cpu::exec::Machine;
        // A minimal v3 machine → its neutral 2-node screen model.
        let story = {
            // Minimal valid v3 header (mirrors the render-test fixtures).
            let mut buf = vec![0u8; 0x0800];
            buf[0x00] = 3;
            buf[0x04] = 0x00; buf[0x05] = 0x40; // high mem base
            buf[0x06] = 0x00; buf[0x07] = 0x40; // initial pc
            buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict
            buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table
            buf[0x0E] = 0x03; buf[0x0F] = 0x00; // globals
            buf[0x08] = 0x04; buf[0x09] = 0x00; // static base
            buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table
            buf[0x0081] = 4; // dict entry size
            buf[0x0040] = 0xba; // quit
            buf
        };
        let mem = zvm::memory::Memory::new(story).expect("minimal v3");
        let machine = Machine::new(mem);
        let model = crate::session::screen_model_from_machine(&machine);
        assert!(is_simple(&model), "Z-machine tree is the simple case");

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.push_transcript("You are in a room.");
        let area = Rect::new(0, 0, 40, 12);

        // Path A: render_story_pane.
        let mut buf_a = Buffer::empty(area);
        let ma = render_story_pane(&model, false, None, &state, area, &mut buf_a);

        // Path B: the exact code render_story_pane replaced.
        let mut buf_b = Buffer::empty(area);
        let used = draw_upper_window(model.grid().unwrap(), false, &state.colors, area, &mut buf_b, state.config.honor_game_colours, &mut Vec::new());
        let tarea = Rect::new(area.x, area.y + used, area.width, area.height.saturating_sub(used));
        let (sb, ms, _, _) = render_transcript(&model.status, None, &state, tarea, &mut buf_b, None);

        assert_eq!(buf_a, buf_b, "the simple path must be byte-identical to the legacy path");
        assert_eq!((ma.scrollbar, ma.max_scroll, ma.viewport_rows), (sb, ms, tarea.height));
    }

    fn graphics_node() -> WinNode {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 1,
            canvas: std::sync::Arc::new(img),
            version: 1,
            upscale: false,
        })
    }

    fn model_with(root: WinNode) -> ScreenModel {
        ScreenModel { root, status: StatusModel::HostManaged, bg: 0, fg: 0, content_size: (0, 0) }
    }

    #[test]
    fn dialog_bounds_returns_frame_when_no_graphics() {
        // A pure-text tree: no graphics → dialogs keep full-frame centering.
        let model = model_with(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }));
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, Rect::new(0, 0, 20, 12), frame), frame);
    }

    #[test]
    fn dialog_bounds_excludes_left_graphics_sidebar_and_spans_map() {
        // Story pane (cols 0..20) = graphics sidebar (cols 0..10) | text buffer
        // (cols 10..20); the map occupies cols 20..40 of the frame. The dialog
        // region must be everything right of the graphics — text + map.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let story_area = Rect::new(0, 0, 20, 12);
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, story_area, frame), Rect::new(10, 0, 30, 12));
    }

    #[test]
    fn dialog_bounds_excludes_top_graphics_band() {
        // Graphics banner (rows 0..3) over the text buffer; no map (TranscriptFull).
        let model = model_with(WinNode::Pair {
            vertical: true,
            split: Split { fixed: 3 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let area = Rect::new(0, 0, 20, 12);
        assert_eq!(dialog_bounds(&model, area, area), Rect::new(0, 3, 20, 9));
    }

    #[test]
    fn dialog_bounds_ignores_graphics_when_story_pane_hidden() {
        // Story pane isn't laid out (empty), so graphics aren't on screen
        // and the dialog centers over the whole frame.
        let model = model_with(WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()),
            second: Box::new(WinNode::Buffer(BufferWindow { primary: true, ..Default::default() })),
        });
        let frame = Rect::new(0, 0, 40, 12);
        assert_eq!(dialog_bounds(&model, Rect::default(), frame), frame);
    }

    #[test]
    fn graphics_leaf_renders_pixels() {
        use ratatui::layout::Rect;
        use ratatui::buffer::Buffer;
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 50, 50, 255]));
        let gw = crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1, upscale: false };
        let picker = ratatui_image::picker::Picker::halfblocks();
        let mut gr = crate::render::graphics::GraphicsRender::default();
        let area = Rect::new(0, 0, 12, 6);
        let mut buf = Buffer::empty(area);
        let style = ratatui::style::Style::default();
        gr.render(&picker, &gw, area, style, &mut buf);
        let has_pixels = (area.top()..area.bottom()).any(|y| (area.left()..area.right())
            .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}")));
        assert!(has_pixels, "graphics canvas should render half-block pixels");
    }

    /// SQ-0325: Counterfeit Monkey is a real Glulx layout (nonzero `content_size`),
    /// so it now routes through the GENERIC tree path — "compliant all the way",
    /// off the simple grid-over-transcript box and onto the spec separator/geometry.
    /// (This flips the old SQ-0303 premise, which kept CM on the simple path to
    /// preserve its framed status border; the generic path renders the game's true
    /// layout instead.) Its tree is still a status grid over the primary buffer — a
    /// vertical Pair with the grid first — but the nonzero extent forces the generic
    /// path. Skips when the (git-ignored) gblorb is absent.
    #[test]
    fn counterfeit_monkey_uses_the_generic_tree_path() {
        use crate::engine::Engine;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/CounterfeitMonkey-11.gblorb");
        if !path.exists() {
            eprintln!("SKIP: stories/CounterfeitMonkey-11.gblorb absent");
            return;
        }
        let blorb = blorb::Blorb::parse(std::fs::read(&path).unwrap()).expect("parse gblorb");
        let image = blorb.executable().expect("exec chunk").1.to_vec();
        let sess = crate::glulx_session::GlulxSession::new(image, 80, 24, true, false, false, (1, 1), None, &[])
            .expect("boot CM");
        let model = sess.screen();
        let (grids, buffers, others) = count_leaves(&model.root);
        assert!(
            !is_simple(&model),
            "CM is a real Glulx layout → generic path (grids={grids}, buffers={buffers}, others={others})"
        );
        // The shape is still a status grid stacked over the primary buffer.
        assert!(
            matches!(&model.root, WinNode::Pair { vertical: true, first, .. } if matches!(first.as_ref(), WinNode::Grid(_))),
            "CM tree is a vertical Pair with the status grid first"
        );
    }

    /// Build the theme used by the separator tests: every app-frame border off, so
    /// the only box-drawing glyphs in the pane are the inter-window separator rules.
    fn frameless_state() -> AppState {
        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = crate::render::paneframe::BorderStyle::None;
        colors.upper_window_border_sides =
            crate::render::paneframe::PaneSides::all(crate::render::paneframe::BorderStyle::None);
        let mut state = AppState::default();
        state.colors = colors;
        state
    }

    /// SQ-0325: a bordered STACKED pair (grid above an inline buffer, `border: true`,
    /// nonzero `content_size` → generic path) draws a horizontal `─` rule filling the
    /// gutter row between the two children, in the themed border colour; the grid sits
    /// above it and the buffer below.
    #[test]
    fn vertical_bordered_pair_draws_horizontal_rule() {
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: true,
                split: Split { fixed: 1 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("STATUS"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        assert!(!is_simple(&model));

        let state = frameless_state();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The gutter row (row 1) between grid (row 0) and buffer (rows 2..) is all `─`.
        assert_eq!(row_text(&buf, 1, 20), "─".repeat(20), "gutter row filled with horizontal rule");
        // In the themed border colour.
        assert_eq!(
            buf.cell((10, 1)).unwrap().style().fg,
            state.colors.theme.get("upper_window_border").style.fg,
            "separator carries the themed window-border colour"
        );
        // Grid content on row 0, buffer below the rule on row 2.
        assert!(row_text(&buf, 0, 20).contains("STATUS"), "grid above the rule: {:?}", row_text(&buf, 0, 20));
        assert_eq!(row_text(&buf, 2, 4), "BODY", "buffer below the rule");
    }

    /// SQ-0325: a bordered SIDE-BY-SIDE pair (grid left of the primary buffer,
    /// `border: true`) draws a vertical `│` rule filling the gutter column between
    /// the children, in the themed border colour; the grid sits left of it.
    #[test]
    fn horizontal_bordered_pair_draws_vertical_rule() {
        let model = ScreenModel {
            root: WinNode::Pair {
                vertical: false,
                split: Split { fixed: 6 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Grid(grid_with("GRID"))),
                second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        assert!(!is_simple(&model));

        let state = frameless_state();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The gutter column (col 6, after the 6-wide grid) is all `│` on every row.
        for y in 0..6 {
            assert_eq!(buf.cell((6, y)).unwrap().symbol(), "│", "vertical rule at split col, row {y}");
        }
        assert_eq!(
            buf.cell((6, 0)).unwrap().style().fg,
            state.colors.theme.get("upper_window_border").style.fg,
            "separator carries the themed window-border colour"
        );
        // Grid content left of the rule (cols < 6) on row 0.
        assert!(row_text(&buf, 0, 6).contains("GRID"), "grid left of the rule: {:?}", row_text(&buf, 0, 6));
    }

    /// SQ-0325: `border: false` on the same shapes draws NO separator glyph — the
    /// children abut with no gutter. Guards that the rule is gated on the flag.
    #[test]
    fn unbordered_pairs_draw_no_separator() {
        for vertical in [true, false] {
            let model = ScreenModel {
                root: WinNode::Pair {
                    vertical,
                    split: Split { fixed: if vertical { 1 } else { 6 } },
                    border: false,
                    key_bg: None,
                    key_fg: None,
                    first: Box::new(WinNode::Grid(grid_with("GRID"))),
                    second: Box::new(WinNode::Buffer(inline_buffer("BODY"))),
                },
                status: StatusModel::HostManaged,
                bg: 0,
                fg: 0,
                content_size: (20, 6),
            };
            let state = frameless_state();
            let area = Rect::new(0, 0, 20, 6);
            let mut buf = Buffer::empty(area);
            render_story_pane(&model, false, None, &state, area, &mut buf);
            for y in 0..6 {
                for x in 0..20 {
                    let s = buf.cell((x, y)).unwrap().symbol();
                    assert!(
                        !"─│".contains(s),
                        "no separator glyph when border:false (vertical={vertical}), found {s:?} at ({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn empty_graphics_neighbour_draws_separator_painted_one_suppresses() {
        // narco frames its story with graphics windows it never paints; the frame
        // must still get our separator rule. Kerkerkruip PAINTS its dividers, so
        // those still suppress our rule (no doubling). (SQ-0340, refines SQ-0332)
        let empty_graphics = || {
            let img = image::RgbaImage::new(9, 57); // opened but never drawn → transparent
            WinNode::Graphics(crate::engine::GraphicsWindow { win: 4, canvas: std::sync::Arc::new(img), version: 1, upscale: false })
        };
        let make = |second: WinNode| ScreenModel {
            root: WinNode::Pair {
                vertical: false, // left/right split → a │ separator
                split: Split { fixed: 10 },
                border: true,
                key_bg: None,
                key_fg: None,
                first: Box::new(WinNode::Buffer(inline_buffer("STORY"))),
                second: Box::new(second),
            },
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (20, 6),
        };
        let state = frameless_state();
        let area = Rect::new(0, 0, 20, 6);
        let has_rule = |m: &ScreenModel| {
            let mut buf = Buffer::empty(area);
            render_story_pane(m, false, None, &state, area, &mut buf);
            (0..6).any(|y| (0..20).any(|x| buf.cell((x, y)).unwrap().symbol() == "\u{2502}"))
        };
        assert!(has_rule(&make(empty_graphics())), "empty graphics neighbour → our separator drawn");
        assert!(!has_rule(&make(graphics_node())), "painted graphics divider → our separator suppressed");
    }

    #[test]
    fn collect_graphics_ids_finds_every_graphics_leaf() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let other = WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(img),
            version: 1,
            upscale: false,
        });
        let tree = WinNode::Pair {
            vertical: false,
            split: Split { fixed: 10 },
            border: false,
            key_bg: None,
            key_fg: None,
            first: Box::new(graphics_node()), // win: 1
            second: Box::new(other),
        };
        let mut ids = std::collections::HashSet::new();
        collect_graphics_ids(&tree, &mut ids);
        assert_eq!(ids, std::collections::HashSet::from([1, 7]));
    }

    /// SQ-0639: v6 painted text is UNTRUSTED game text and it can carry control
    /// characters — `print_unicode` (ZMSD EXT:0x0B) prints any codepoint a story
    /// asks for, and a story-supplied Unicode translation table can map ZSCII 155+
    /// to one. Handing such a run straight to `Buffer::set_stringn` does not draw a
    /// control char, it DROPS it — and every glyph after it shifts a column left,
    /// which for pixel-positioned v6 runs is exactly the alignment the path exists
    /// to preserve. Blanking to a space keeps the columns. Pinned in both
    /// `honor_game_colours` modes.
    #[test]
    fn painted_game_text_blanks_control_chars_instead_of_shifting_the_run() {
        let colors = crate::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 20, 3);
        let read = |buf: &Buffer, y: u16, n: u16| -> String {
            (0..n).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        for honor in [true, false] {
            // A painted run with a BEL in the middle, at native (row 0, col 0).
            let t = crate::engine::PxText { y: 1, x: 1, text: "AB\u{7}CD".into(), style: 0, fg: 0, bg: 0 };
            let runs: Vec<&crate::engine::PxText> = vec![&t];
            let mut buf = Buffer::empty(area);
            draw_painted_screen(
                &runs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), honor, &colors, &[], 640,
            );
            assert_eq!(read(&buf, 0, 5), "AB CD", "honor={honor}: the control char blanks, D stays in column 4");

            // The anchored status band takes its groups from the same runs.
            let mut band = Buffer::empty(area);
            assert!(place_anchored_row(
                &mut band, area, 0, "L\u{1}T", "", "R\u{2}T", ratatui::style::Style::default()
            ));
            assert_eq!(read(&band, 0, 3), "L T", "honor={honor}: LEFT group keeps its width");
            assert_eq!(read(&band, 0, area.width), format!("{:<17}R T", "L T"), "honor={honor}: RIGHT stays flush right");
        }
    }

    #[test]
    fn collect_graphics_ids_walks_layered_like_collect_graphics_rects() {
        // SQ-0637: a v6 composite is a `Layered` root. Missing that arm reported NO
        // live windows for such a frame, so `retain_live` cleared the whole protocol
        // cache every frame — a full re-encode, and under kitty a fresh id per frame
        // with the previous ones never deleted. The id walk must cover exactly what
        // the rect walk covers.
        let pane = Rect::new(0, 0, 40, 20);
        let pw = |win: u32, x: u16, y: u16| PositionedWindow {
            x,
            y,
            w: 6,
            h: 4,
            x_px: x * 8,
            y_px: y * 16,
            w_px: 48,
            h_px: 64,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win,
                canvas: std::sync::Arc::new(image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]))),
                version: 1,
                upscale: false,
            }),
        };
        let text = PositionedWindow { node: WinNode::Buffer(inline_buffer("STORY")), ..pw(9, 0, 8) };
        let tree = WinNode::Layered(vec![pw(3, 0, 0), text, pw(5, 10, 2)]);

        let mut ids = std::collections::HashSet::new();
        collect_graphics_ids(&tree, &mut ids);
        assert_eq!(ids, std::collections::HashSet::from([3, 5]), "every layered graphics leaf is live");

        let mut rects = Vec::new();
        collect_graphics_rects(&tree, pane, &mut rects);
        assert_eq!(rects.len(), ids.len(), "the id walk and the rect walk see the same windows");
    }

    /// v6 layered composite (Phase 1b): a full-area solid graphics window
    /// (background) with a small grid (foreground) drawn on top. The grid's
    /// one non-blank cell must land at its absolute rect; a BLANK grid cell
    /// must leave the graphics layer's colour showing through — cell-text-wins.
    #[test]
    fn layered_composite_draws_zorder_with_cell_text_wins() {
        use ratatui::style::Color;

        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // No picker: this test exercises the Phase 1b cell composite fallback.
        // With a picker, Phase 1c takes over `Layered` and draws one pixel image
        // instead (see `layered_composite_*` picker-path coverage elsewhere).

        // Background: a full-area solid-colour graphics window.
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 20, 30, 255]));
        let background = PositionedWindow {
            x: 0,
            y: 0,
            w: 10,
            h: 6,
            x_px: 0,
            y_px: 0,
            w_px: 80,
            h_px: 48,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 1,
                canvas: std::sync::Arc::new(img),
                version: 1,
                upscale: false,
            }),
        };

        // Foreground: a 3x2 grid, positioned at (2,2), with a single non-blank cell.
        let mut grid = GridWindow::default();
        grid.resize(2, 3);
        grid.active_rows = 2;
        grid.put(1, 1, 'X', 0);
        let foreground = PositionedWindow {
            x: 2,
            y: 2,
            w: 3,
            h: 2,
            x_px: 16,
            y_px: 16,
            w_px: 24,
            h_px: 16,
            left_margin: 0,
            right_margin: 0,
            node: WinNode::Grid(grid),
        };

        let model = ScreenModel {
            root: WinNode::Layered(vec![background, foreground]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (10, 6),
        };

        let area = Rect::new(0, 0, 10, 6);
        let mut buf = Buffer::empty(area);
        render_story_pane(&model, false, None, &state, area, &mut buf);

        // The grid's non-blank cell is drawn at its absolute rect (2,2).
        assert_eq!(buf.cell((2, 2)).unwrap().symbol(), "X", "grid glyph at its absolute cell");

        // A blank grid cell (grid col 2, row 1 → absolute (3,2)) is transparent:
        // the background graphics colour shows through instead of a grid fill.
        assert_eq!(
            buf.cell((3, 2)).unwrap().style().bg,
            Some(Color::Rgb(10, 20, 30)),
            "blank grid cell is transparent — graphics layer shows through"
        );
    }

    /// A synthetic v6 `Layered` model (native 320×200: a full-area opaque chrome
    /// graphics window + a primary story `Buffer` at Zork0's win0 box), for the
    /// Lane H hybrid-branch tests.
    fn hybrid_v6_model() -> ScreenModel {
        // Native-sized opaque chrome so build_chrome_canvas yields a real ring.
        let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        // Story: the primary buffer at the win0 box (43,39,234,160).
        let story = PositionedWindow {
            x: 5, y: 4, w: 29, h: 20, x_px: 43, y_px: 39, w_px: 234, h_px: 160,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![chrome, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn hybrid_deep_status_outside_story_box_keeps_the_ring() {
        // SQ-0494: Arthur paints its status bar as reverse px_text runs at a deep
        // native row (12 on the real 640×400 screen) ABOVE its story buffer — with
        // graphics windows carrying the top image panel and side borders. That is
        // ordinary gameplay chrome, NOT a menu takeover: the ring path must be
        // kept (the status text belongs to the pixel ring, so it must NOT be
        // painted into the terminal cells the way a routed menu screen is).
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("HELLO STORY WORLD");

        let chrome_img = image::RgbaImage::from_pixel(320, 200, image::Rgba([40, 30, 20, 255]));
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 40, h: 25, x_px: 0, y_px: 0, w_px: 320, h_px: 200,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        // Status grid: a non-blank run at native row 6 (deep, ≥ STATUS_BAND_ROWS)
        // but ABOVE the story buffer, which starts at row 7 (y_px 112).
        let status = PositionedWindow {
            x: 0, y: 6, w: 40, h: 1, x_px: 0, y_px: 96, w_px: 320, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                fill: None,
                cols: 40, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![crate::engine::PxText {
                    y: 97, x: 1, text: "Score: 0".into(), style: 1, fg: 0, bg: 0,
                }],
            }),
        };
        let story = PositionedWindow {
            x: 0, y: 7, w: 40, h: 10, x_px: 0, y_px: 112, w_px: 320, h_px: 80,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        let model = ScreenModel {
            root: WinNode::Layered(vec![chrome, status, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        };
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let metrics = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );
        assert!(metrics.is_some(), "ring path taken (it returns inset metrics)");
        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect::<String>() + "\n")
            .collect();
        assert!(
            !screen.contains("Score: 0"),
            "deep-but-outside-story status chrome stays in the pixel ring, not cells:\n{screen}"
        );
    }

    #[test]
    fn hybrid_renders_story_as_terminal_text_in_an_inset_viewport() {
        // Hybrid + a picker: the Layered arm draws the chrome ring and renders the
        // story window as REAL terminal text (via render_transcript) into an inset
        // viewport — so render_node returns Some(metrics) and the transcript
        // publishes its geometry inside (strictly smaller than) the full pane.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("HELLO STORY WORLD");

        let model = hybrid_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );
        let m = m.expect("hybrid story viewport returns primary-buffer metrics");
        assert!(m.viewport_rows > 0, "story viewport has rows");

        // The transcript rendered as terminal cells into an inset viewport.
        let geom = state.transcript_geom.get().expect("hybrid renders the transcript as terminal cells");
        let vp = geom.area;
        assert!(vp.width < area.width && vp.height < area.height, "viewport is inset inside the chrome ring: {vp:?}");
        assert!(vp.x >= area.x && vp.y >= area.y && vp.right() <= area.right() && vp.bottom() <= area.bottom(),
            "viewport stays inside the pane: {vp:?}");
    }

    #[test]
    fn hybrid_menu_screen_renders_coherent_all_text_with_transcript() {
        // SQ-0484: Shogun's boot menu keeps window 0 (the story buffer) open AND
        // paints its three menu items as DEEP chrome runs (native rows ≥
        // STATUS_BAND_ROWS). The old ring+viewport path split that menu across the
        // raster pixel ring (items mapping above the terminal viewport → rendered
        // as pixel art) and the terminal overlay (items inside it → terminal text),
        // giving the mixed "first option raster, rest terminal text" screen. A menu
        // screen must instead route to the cell path — the story transcript plus the
        // menu painted over it as ONE coherent all-text screen, matching the
        // frameless path — so all three items are terminal cells and the transcript
        // ("You may choose to:") is preserved.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Hybrid;
        state.push_transcript("You may choose to:");

        let mut model = hybrid_v6_model();
        // A chrome grid whose pixel runs sit DEEP (native rows 8/9/10, ≥
        // STATUS_BAND_ROWS) inside the story box (native y 39..199 → the 8×16 cell
        // rows land at (y-1)/16). Distinct rows, like Shogun's real 21/22/23.
        let menu = PositionedWindow {
            x: 12, y: 8, w: 1, h: 3, x_px: 100, y_px: 129, w_px: 1, h_px: 48,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                fill: None,
                cols: 1, rows: 3, cells: vec![], active_rows: 3, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText { y: 129, x: 101, text: "START the game".into(), style: 0, fg: 0, bg: 0 },
                    crate::engine::PxText { y: 145, x: 101, text: "RESTORE a saved game".into(), style: 0, fg: 0, bg: 0 },
                    crate::engine::PxText { y: 161, x: 101, text: "QUIT the game".into(), style: 0, fg: 0, bg: 0 },
                ],
            }),
        };
        if let WinNode::Layered(items) = &mut model.root {
            items.push(menu);
        }
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let _ = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );
        // A menu screen publishes a full terminal transcript geometry (the cell
        // path), NOT a raster/hybrid image — so the transcript renders as real cells.
        let row_text = |y: u16| -> String {
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
        };
        let screen: String = (0..area.height).map(|y| row_text(y) + "\n").collect();
        // The transcript prompt is preserved (dropped by a painted-only path).
        assert!(screen.contains("You may choose to:"), "transcript prompt preserved, screen:\n{screen}");
        // All three items render as terminal text on their distinct deep rows,
        // placed relative to the STORY BOX they are painted inside (SQ-0697): this
        // model's story window starts at native y=39 — row 2 — with no chrome above
        // it, so the box takes the pane's top row and its contents come with it. The
        // items' native rows 8/9/10 therefore land two rows up, at 6/7/8; stamping
        // them at absolute native rows instead would tear a menu away from the
        // transcript it is painted over.
        assert_eq!(row_text(6).trim(), "START the game", "row 6 is the START item, screen:\n{screen}");
        assert_eq!(row_text(7).trim(), "RESTORE a saved game", "row 7 is the RESTORE item");
        assert_eq!(row_text(8).trim(), "QUIT the game", "row 8 is the QUIT item");
    }

    /// SQ-0515: a chrome grid window carrying `px_texts`, for the flood discriminator.
    fn flood_probe_window(w_px: u16, runs: Vec<crate::engine::PxText>) -> PositionedWindow {
        PositionedWindow {
            x: 0, y: 0, w: 1, h: 1, x_px: 0, y_px: 0, w_px, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                fill: None,
                cols: 1, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false, px_texts: runs,
            }),
        }
    }

    #[test]
    fn painted_screen_floods_only_full_width_reverse_rows() {
        use ratatui::style::Modifier;
        // Native 640px = 80 cells. A FULL-width window (w_px=640) with an all-reverse
        // row floods edge to edge; a NARROW window (w_px=169, ~26%) with a reverse row
        // stays a text-width block; a full-width row with a MIXED reverse/non-reverse
        // run set is NOT all-reverse, so it stays text-width too.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let native_w = 640u16;

        let full = flood_probe_window(640, vec![
            // Row 0: single reversed run → floods.
            crate::engine::PxText { y: 1, x: 1, text: "TITLE".into(), style: 1, fg: 0, bg: 0 },
            // Row 1: one reversed + one non-reversed run → mixed, does NOT flood.
            crate::engine::PxText { y: 17, x: 1, text: "LEFT".into(), style: 1, fg: 0, bg: 0 },
            crate::engine::PxText { y: 17, x: 401, text: "RIGHT".into(), style: 0, fg: 0, bg: 0 },
        ]);
        let narrow = flood_probe_window(169, vec![
            // Row 2: reversed run in a narrow window → text-width block, no flood.
            crate::engine::PxText { y: 33, x: 1, text: "SEL".into(), style: 1, fg: 0, bg: 0 },
        ]);
        let chrome: Vec<&PositionedWindow> = vec![&full, &narrow];
        let runs: Vec<&crate::engine::PxText> = chrome
            .iter()
            .filter_map(|it| match &it.node {
                WinNode::Grid(g) => Some(g.px_texts.iter()),
                _ => None,
            })
            .flatten()
            .collect();

        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        draw_painted_screen(&runs, 0..u16::MAX, 0, area, &mut buf, base, true, &colors, &chrome, native_w);

        let reversed_count = |y: u16| -> u16 {
            (0..area.width).filter(|&x| buf.cell((x, y)).unwrap().modifier.contains(Modifier::REVERSED)).count() as u16
        };
        // Row 0: full-width all-reverse → flooded edge to edge.
        assert_eq!(reversed_count(0), area.width, "full-width all-reverse row floods every cell");
        // Row 1: full-width but MIXED reverse → only the "LEFT" glyphs reversed, no flood.
        assert!(reversed_count(1) > 0 && reversed_count(1) < area.width, "mixed-reverse row is not flooded: {} reversed", reversed_count(1));
        // Row 2: narrow window reverse → only "SEL" (3 cells) reversed, no flood.
        assert_eq!(reversed_count(2), 3, "narrow-window reverse row stays a text-width block");
    }

    /// A v6 Layered model whose chrome is fully TRANSPARENT, leaving the story
    /// window's box as a clear raster interior (the opaque `hybrid_v6_model` chrome
    /// insets `story_clear_native` to nothing). Story box native (43,39,234,160) →
    /// 29×20 raster cells (a 19-row body budget).
    fn raster_v6_model() -> ScreenModel {
        // Authentic 640×400 unit geometry (SQ-0479): the story window's 320px
        // height quantizes to 320/FONT_H(16) = 20 raster rows.
        let chrome_img = image::RgbaImage::new(640, 400); // all alpha 0 (transparent)
        let chrome = PositionedWindow {
            x: 0, y: 0, w: 80, h: 25, x_px: 0, y_px: 0, w_px: 640, h_px: 400,
            left_margin: 0, right_margin: 0,
            node: WinNode::Graphics(crate::engine::GraphicsWindow {
                win: 7, canvas: std::sync::Arc::new(chrome_img), version: 1, upscale: false,
            }),
        };
        let story = PositionedWindow {
            x: 10, y: 4, w: 58, h: 20, x_px: 86, y_px: 78, w_px: 468, h_px: 320,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![chrome, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn raster_mode_publishes_scroll_geometry() {
        // SQ-0455: raster mode is still one rasterized pixel image (draw_v6_canvas),
        // but it now REPORTS the story box's scroll geometry so the shared scroll
        // keybindings and the [more] pager (SQ-0404) engage — replacing the old
        // behavior where the raster path returned None and published no geometry.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Raster;
        // 40 short lines overflow the 19-row body → real scroll capacity.
        for k in 0..40 {
            state.push_transcript(&format!("L{k}"));
        }

        let model = raster_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );
        let m = m.expect("raster path now reports story-box scroll metrics");
        assert_eq!(m.viewport_rows, 19, "story box is 20 raster rows (320px/FONT_H 16) minus the input line");
        assert_eq!(m.total_rows, 40, "all 40 wrapped transcript rows counted");
        assert_eq!(m.max_scroll, 21, "40 total - 19 body");

        // Geometry is published (the raster grid is pixel-scaled, so area is the
        // whole pane — mouse mapping is approximate, scroll math is exact).
        let geom = state.transcript_geom.get().expect("raster mode publishes scroll geometry");
        assert_eq!(geom.total_rows, 40);
        assert_eq!(geom.first_abs_row, 21, "offset 0 → newest body at the bottom (40 - 19)");
    }

    #[test]
    fn v6_raster_gen_stable_when_idle_bumps_on_change() {
        // SQ-0469: the generation gate skips the whole rebuild+encode when nothing
        // changed, so an idle frame must produce an identical key while every real
        // input change must alter it.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        let picker = ratatui_image::picker::Picker::halfblocks();
        state.push_transcript("You are in a maze.");
        let area = Rect::new(0, 0, 40, 25);

        let model = raster_v6_model();
        let items = match model.root {
            WinNode::Layered(v) => v,
            other => panic!("expected Layered, got {other:?}"),
        };

        let base = v6_raster_gen(&items, &state, area, &picker);
        // Idle: recomputing with no change is identical → the gate skips the frame.
        assert_eq!(base, v6_raster_gen(&items, &state, area, &picker), "idle frame → same key");

        // A v6 run change (here a picture repaint bumps its version stamp).
        let mut mutated = items.clone();
        if let WinNode::Graphics(g) = &mut mutated[0].node {
            g.version = g.version.wrapping_add(1);
        }
        assert_ne!(base, v6_raster_gen(&mutated, &state, area, &picker), "a v6 window change bumps the key");

        // A transcript append.
        let mut s2 = AppState::default();
        s2.colors = crate::colors::ColorScheme::terminal_default();
        s2.push_transcript("You are in a maze.");
        s2.push_transcript("A grue lurks nearby.");
        assert_ne!(base, v6_raster_gen(&items, &s2, area, &picker), "new transcript output bumps the key");

        // A keystroke on the live input line.
        state.input.value.push('x');
        assert_ne!(base, v6_raster_gen(&items, &state, area, &picker), "an input-line keystroke bumps the key");
        state.input.value.clear();

        // A pane resize.
        assert_ne!(base, v6_raster_gen(&items, &state, Rect::new(0, 0, 41, 25), &picker), "a resize bumps the key");

        // Scrolling the transcript back.
        state.transcript_scroll = 3;
        assert_ne!(base, v6_raster_gen(&items, &state, area, &picker), "a scroll change bumps the key");
    }

    /// A synthetic v6 `Layered` model for the frameless-mode tests: a chrome
    /// `Grid` carrying one status px-run at native (1,1) → cell (0,0), plus a
    /// primary story `Buffer` one native row below it. No decorative graphics
    /// window. Pixel geometry is the authentic 8×16 v6 text cell (SQ-0479), so
    /// the status window really does occupy the row ABOVE the story — the
    /// relation the frameless band split reads (SQ-0549).
    fn frameless_v6_model() -> ScreenModel {
        let status = PositionedWindow {
            x: 0, y: 0, w: 40, h: 1, x_px: 0, y_px: 0, w_px: 320, h_px: 16,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                fill: None,
                cols: 40, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText { y: 1, x: 1, text: "SCORE 10".into(), style: 0, fg: 0, bg: 0 },
                ],
            }),
        };
        let story = PositionedWindow {
            x: 0, y: 1, w: 40, h: 24, x_px: 0, y_px: 16, w_px: 320, h_px: 384,
            left_margin: 0, right_margin: 0,
            node: WinNode::Buffer(BufferWindow { primary: true, ..Default::default() }),
        };
        ScreenModel {
            root: WinNode::Layered(vec![status, story]),
            status: StatusModel::HostManaged,
            bg: 0,
            fg: 0,
            content_size: (40, 25),
        }
    }

    #[test]
    fn frameless_renders_full_pane_transcript_with_status_band_and_no_graphics() {
        // SQ-0461: `v6_render = "frameless"` deliberately skips the pixel chrome
        // (both the hybrid ring and the raster composite) even with a picker
        // present, and presents the story as a normal full-pane terminal
        // transcript with the chrome text collapsed to a compact status band.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        // A picker is present (images enabled) — frameless must STILL bypass the
        // pixel paths and use the terminal transcript.
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Frameless;
        state.push_transcript("HELLO STORY WORLD");

        let model = frameless_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let m = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );
        let m = m.expect("frameless returns the primary-buffer transcript metrics");

        // The transcript occupies the FULL pane below the one-row status band —
        // NOT an inset chrome-ring viewport (hybrid) and NOT a pixel raster. The
        // transcript always reserves the rightmost column as a scrollbar gutter,
        // so a full-pane body is `area.width - 1` wide (vs hybrid's much-narrower
        // inset viewport).
        let geom = state.transcript_geom.get().expect("frameless publishes transcript geometry");
        let vp = geom.area;
        assert_eq!(vp.x, area.x, "transcript is flush to the left pane edge (not inset)");
        assert_eq!(vp.width, area.width - 1, "transcript spans the full pane width minus the scrollbar gutter");
        assert_eq!(vp.y, area.y + 1, "transcript starts below the 1-row status band");
        assert_eq!(m.viewport_rows, area.height - 1, "metrics report the full-pane body height below the band");

        // The whole pane rendered as real terminal cells: the status run sits in
        // the top row and the story text renders as selectable text below it.
        let screen: String = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect::<String>() + "\n")
            .collect();
        assert!(screen.contains("SCORE 10"), "status band renders as terminal text, screen:\n{screen}");
        assert!(screen.contains("HELLO STORY WORLD"), "story renders as a full-pane transcript, screen:\n{screen}");
    }

    #[test]
    fn frameless_publishes_a_click_map_covering_the_pane() {
        // SQ-0532/A-F4: the cell path (frameless, and the no-picker fallback)
        // draws no game image, so it used to record NO click map at all — v6
        // mouse input was dead there while raster and hybrid both worked. It now
        // records the proportional pane→native map, and a click maps into the
        // game-pixel rect the clicked cell stands for.
        let mut state = AppState::default();
        state.colors = crate::colors::ColorScheme::terminal_default();
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = crate::config::V6RenderMode::Frameless;
        state.push_transcript("HELLO STORY WORLD");

        let model = frameless_v6_model();
        let area = Rect::new(0, 0, 40, 25);
        let mut buf = Buffer::empty(area);
        let mut links = Vec::new();
        let _ = render_node(
            &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
        );

        let map = state
            .graphics_render
            .borrow()
            .last_v6_map
            .expect("frameless publishes a v6 click map");
        // The model's native extent is its 320x200 game-pixel screen.
        let (nw, nh) = crate::render::v6_layout::native_extent(match &model.root {
            WinNode::Layered(items) => items,
            _ => unreachable!("frameless_v6_model is Layered"),
        });
        assert_eq!((map.native_w, map.native_h), (nw, nh));

        // Top-left cell → the top-left game pixel (1-based origin, ZMSD §8.8.1).
        let (gx, gy) = map.map_click(area.x, area.y).expect("a click inside the pane maps");
        assert!(gx <= nw / area.width + 1 && gy <= nh / area.height + 1,
            "top-left cell maps into the top-left game-pixel cell, got ({gx},{gy})");
        // A known interior cell maps into the game-pixel rect it stands for: cell
        // (col, row) covers native x in [col/W, (col+1)/W) of the screen width.
        let (col, row) = (area.x + 30, area.y + 20);
        let (gx, gy) = map.map_click(col, row).expect("interior click maps");
        let (lo_x, hi_x) = (nw as u32 * 30 / 40, nw as u32 * 31 / 40);
        let (lo_y, hi_y) = (nh as u32 * 20 / 25, nh as u32 * 21 / 25);
        assert!((lo_x..=hi_x).contains(&(gx as u32 - 1)), "x {gx} in {lo_x}..={hi_x}");
        assert!((lo_y..=hi_y).contains(&(gy as u32 - 1)), "y {gy} in {lo_y}..={hi_y}");
        // Outside the pane is still a miss (the app falls back to selection).
        assert_eq!(map.map_click(area.right() + 2, area.y), None);
    }

    #[test]
    fn frameless_no_images_equals_cell_fallback() {
        // With `--no-images` (no picker) frameless must be byte-identical to the
        // classic cell fallback: same full-pane transcript + status band. Render
        // the SAME model once as the default (no picker → cell fallback) and once
        // as frameless (no picker) and assert the buffers match.
        let render = |mode: crate::config::V6RenderMode| {
            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.game_picker = None; // --no-images
            state.config.v6_render = mode;
            state.push_transcript("HELLO STORY WORLD");
            let model = frameless_v6_model();
            let area = Rect::new(0, 0, 40, 25);
            let mut buf = Buffer::empty(area);
            let mut links = Vec::new();
            let _ = render_node(
                &model.root, &model.status, false, None, &state, area, &mut buf, None, &mut links, &state.colors,
            );
            (0..area.height)
                .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().to_string()).collect::<String>() + "\n")
                .collect::<String>()
        };
        assert_eq!(
            render(crate::config::V6RenderMode::Hybrid),
            render(crate::config::V6RenderMode::Frameless),
            "with no picker, frameless equals the classic cell fallback"
        );
    }

    // ── Anchored status band (SQ-0467) ──────────────────────────────────────────

    /// One px-run at native pixel `(x, y)` (1-based) carrying `text`.
    fn run(x: u16, y: u16, text: &str) -> crate::engine::PxText {
        crate::engine::PxText { y, x, text: text.into(), style: 0, fg: 0, bg: 0 }
    }

    /// Render `runs` as an anchored band over a `w`-cell pane at native width
    /// `ncols` cells, returning the top row's text (trailing spaces trimmed off
    /// the right only via the caller) plus the raw buffer for column probing.
    fn band_row(runs: &[crate::engine::PxText], ncols: u32, w: u16) -> (String, u16) {
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, w, 6);
        let mut buf = Buffer::empty(area);
        let style = ratatui::style::Style::default();
        let colors = crate::colors::ColorScheme::terminal_default();
        let rows_used = draw_anchored_status_band(&refs, ncols, 4, area, &mut buf, style, true, &colors);
        let text: String = (0..w).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect();
        (text, rows_used)
    }

    #[test]
    fn anchored_band_shogun_shape_left_center_right() {
        // Native 40-cell screen: a location/name run at the far left, a centered
        // title, and two right-side status runs. Left flush col 0, title centered,
        // the two right runs two-space joined and ending flush at the last column.
        let runs = vec![
            run(1, 1, "Shogun"),           // start col 0 → LEFT
            run(129, 1, "The Tale"),       // start col 16, end col 24 → CENTER
            run(233, 1, "Score: 0"),       // start col 29, end col 37 → RIGHT
            run(281, 1, "Moves: 1"),       // start col 35, end col 43 → RIGHT
        ];
        let (row, rows_used) = band_row(&runs, 40, 80);
        assert_eq!(rows_used, 1);
        assert!(row.starts_with("Shogun"), "left run flush at col 0: {row:?}");
        // Right group: two runs joined by exactly two spaces, ending flush right.
        assert!(row.trim_end().ends_with("Score: 0  Moves: 1"), "right group joined + flush: {row:?}");
        assert_eq!(row.chars().count(), 80);
        assert_eq!(&row[row.len() - "Moves: 1".len()..], "Moves: 1", "right group ends at the last column");
        // Title centered within ±1 of the pane centre.
        let title_start = row.find("The Tale").expect("centered title present");
        let expected = (80 - "The Tale".chars().count()) / 2;
        assert!((title_start as i32 - expected as i32).abs() <= 1, "title centered (at {title_start}, want ~{expected})");
    }

    #[test]
    fn anchored_band_zork0_shape_location_and_right_status() {
        // Location at the left, score/moves at the right — no centre group.
        let runs = vec![
            run(9, 1, "West of House"),   // start col 1 → LEFT
            run(241, 1, "Score: 0"),      // → RIGHT
            run(297, 1, "Moves: 3"),      // → RIGHT
        ];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with("West of House"), "location flush left: {row:?}");
        assert!(row.trim_end().ends_with("Score: 0  Moves: 3"), "score/moves joined + flush right: {row:?}");
        assert_eq!(&row[row.len() - "Moves: 3".len()..], "Moves: 3");
    }

    #[test]
    fn anchored_band_narrow_pane_priority_and_truncation() {
        // A 28-col pane: LEFT stays intact, RIGHT truncates from its left edge to
        // keep a space from LEFT, CENTER drops because it can't fit between them.
        let runs = vec![
            run(1, 1, "A Fairly Long Location"), // 22 chars → LEFT (col 0)
            run(129, 1, "Title"),                // CENTER (dropped, no room)
            run(281, 1, "Moves: 100"),           // RIGHT (10 chars, must truncate)
        ];
        let (row, _) = band_row(&runs, 40, 28);
        assert_eq!(row.chars().count(), 28);
        assert!(row.starts_with("A Fairly Long Location"), "LEFT intact: {row:?}");
        assert!(!row.contains("Title"), "CENTER dropped when it can't fit: {row:?}");
        // RIGHT truncated from the left, still flush to the last column, and never
        // overwriting LEFT: a space separates them.
        assert_eq!(row.chars().nth(22), Some(' '), "≥1 space between LEFT and RIGHT");
        assert!(row.ends_with(|c: char| c != ' '), "RIGHT still flush at the last column: {row:?}");
        // Only the last 5 cols hold RIGHT's tail (28 - 22 - 1 = 5 chars survive).
        let tail: String = row.chars().skip(23).collect();
        assert_eq!(tail, ": 100", "RIGHT truncated from its left edge to the fitting tail");
    }

    /// SQ-0717: a line the game centred on its own screen stays centred, however
    /// far left it begins. Shogun's frozen title header (SQ-0697) is the case —
    /// nine cursor-centred lines that the thirds rule sorted by their START, so the
    /// long ones began left of the left-third boundary and flushed to col 0 while
    /// the shortest ended past the right two-thirds and flushed right.
    #[test]
    fn anchored_band_keeps_a_line_the_game_centred() {
        // Shogun's own columns, on its 640px (80-cell) screen.
        let lines: [(u16, &str); 3] = [
            (297, "SHOGUN"),                                             // col 37, well inside
            (105, "Original Literary Work Copyright 1975 by James Clavell"), // col 13 → was LEFT
            (209, "IBM Interpreter version 6.65"),                       // ends col 54 → was RIGHT
        ];
        for w in [80u16, 120] {
            for (x, text) in lines {
                let (row, _) = band_row(&[run(x, 1, text)], 80, w);
                let at = row.find(text).unwrap_or_else(|| panic!("{text:?} painted at {w} cols: {row:?}"));
                let want = (w as usize - text.chars().count()) / 2;
                assert!(
                    (at as i32 - want as i32).abs() <= 1,
                    "{text:?} stays centred at {w} cols (at {at}, want ~{want}): {row:?}"
                );
            }
        }
    }

    /// …and the centring exemption does not loosen a real bar. A field that begins
    /// at the screen's left edge is LEFT even when its right margin happens to
    /// match, and edge-anchored status fields keep their thirds classification.
    #[test]
    fn anchored_band_centring_exemption_spares_edge_anchored_fields() {
        // A rule drawn from col 0: margins 0 and 4 — not centred, still LEFT.
        let bar = "=".repeat(36);
        let (row, _) = band_row(&[run(1, 1, &bar)], 40, 80);
        assert!(row.starts_with(&bar), "an edge-anchored rule is not 'centred': {row:?}");
        // Location left, score/moves right — the classic bar, unmoved.
        let runs = vec![run(9, 1, "West of House"), run(241, 1, "Score: 0"), run(297, 1, "Moves: 3")];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with("West of House"), "location still flush left: {row:?}");
        assert!(row.trim_end().ends_with("Score: 0  Moves: 3"), "score/moves still flush right: {row:?}");
    }

    /// SQ-0712: the band is measured before the story area is sized and painted
    /// after the erase fills, so `anchored_band_rows` has to agree with what
    /// `draw_anchored_status_band` actually uses — a drift between them mis-sizes
    /// the transcript or strands the bar off-pane.
    #[test]
    fn anchored_band_measurement_matches_the_draw() {
        let cases: Vec<(Vec<crate::engine::PxText>, u16, u16)> = vec![
            // One bar row at the top of a 4-row band.
            (vec![run(1, 1, "Loc"), run(281, 1, "Moves: 1")], 4, 6),
            // Two rows, one apart.
            (vec![run(1, 1, "Row0"), run(1, 17, "Row1")], 4, 6),
            // A gap row in the middle still counts toward the span.
            (vec![run(1, 1, "Row0"), run(1, 49, "Row3")], 4, 6),
            // Arthur's shape: nothing until native row 12, band 13 deep.
            (vec![run(33, 193, "Churchyard")], 13, 6),
            // Blank-only runs paint nothing and measure nothing.
            (vec![run(129, 1, "   ")], 4, 6),
            // Nothing at all.
            (vec![], 4, 6),
            // The span is clamped to a pane shorter than the band.
            (vec![run(1, 1, "Row0"), run(1, 81, "Row5")], 8, 3),
        ];
        for (runs, band_rows, pane_h) in cases {
            let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
            let area = Rect::new(0, 0, 80, pane_h);
            let mut buf = Buffer::empty(area);
            let colors = crate::colors::ColorScheme::terminal_default();
            let drawn = draw_anchored_status_band(
                &refs, 40, band_rows, area, &mut buf, ratatui::style::Style::default(), true, &colors,
            );
            assert_eq!(
                anchored_band_rows(&refs, band_rows, pane_h),
                drawn,
                "measured rows must equal drawn rows for {:?} (band {band_rows}, pane {pane_h})",
                runs.iter().map(|t| (t.y, t.x, &t.text)).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn anchored_band_multi_row() {
        // Runs on native rows 0 and 1 (y=1 and y=17, one 16px cell apart) each
        // render on their own band row, and rows_used reports 2.
        let runs = [run(1, 1, "Row0Left"), run(233, 1, "Score: 0"), run(1, 17, "Row1Left")];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        let colors = crate::colors::ColorScheme::terminal_default();
        let rows_used = draw_anchored_status_band(&refs, 40, 4, area, &mut buf, ratatui::style::Style::default(), true, &colors);
        assert_eq!(rows_used, 2, "two native rows populated");
        let r0: String = (0..80).map(|x| buf.cell((x, 0)).unwrap().symbol().to_string()).collect();
        let r1: String = (0..80).map(|x| buf.cell((x, 1)).unwrap().symbol().to_string()).collect();
        assert!(r0.starts_with("Row0Left"), "row 0 left: {r0:?}");
        assert!(r0.trim_end().ends_with("Score: 0"), "row 0 right flush: {r0:?}");
        assert!(r1.starts_with("Row1Left"), "row 1 left: {r1:?}");
    }

    #[test]
    fn anchored_band_wide_run_counts_left_not_stretched() {
        // A full-width bar (spans most of the row) anchors LEFT at col 0 rather
        // than being treated as a centred title.
        let bar = "=".repeat(36);
        let runs = vec![run(1, 1, &bar)];
        let (row, _) = band_row(&runs, 40, 80);
        assert!(row.starts_with(&bar), "wide bar flush at col 0, not centred: {row:?}");
    }

    #[test]
    fn anchored_band_skips_blank_runs() {
        // A whitespace-only run must not drag a group around or count as painted.
        let runs = vec![run(129, 1, "   ")];
        let (row, rows_used) = band_row(&runs, 40, 80);
        assert_eq!(rows_used, 0, "blank-only band paints nothing");
        assert!(row.trim().is_empty(), "no text painted: {row:?}");
    }

    #[test]
    fn anchored_band_tiny_pane_no_panic() {
        // Width 1 and 2 must not panic and LEFT still wins.
        for w in [1u16, 2] {
            let runs = vec![run(1, 1, "Loc"), run(281, 1, "Moves: 1")];
            let (_row, _) = band_row(&runs, 40, w);
        }
    }

    /// A reverse-video px-run at native pixel `(x, y)` (1-based) carrying `text`.
    fn rev_run(x: u16, y: u16, text: &str) -> crate::engine::PxText {
        crate::engine::PxText { y, x, text: text.into(), style: 1, fg: 0, bg: 0 }
    }

    #[test]
    fn painted_screen_fills_reverse_video_gaps_between_words() {
        use ratatui::style::Modifier;
        // SQ-0484 defect 2: a highlighted (reverse-video) menu item paints each
        // word AND each inter-word space as a SEPARATE run. Dropping the blank
        // runs left the selection bar reversed behind the glyphs but not the gaps
        // ("moth-eaten"). The reversed blank runs must now be stamped, so the whole
        // bar reads as one solid reverse block. A NON-reverse blank stays a no-op.
        //
        // Row 1 (native y=17 → cell row 1): "GO" at cols 0..2, a reverse space at
        // col 2, "IN" at cols 3..5 — the gap cell (2) must carry REVERSED.
        let runs = [
            rev_run(1, 17, "GO"),
            rev_run(17, 17, " "),
            rev_run(25, 17, "IN"),
        ];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let colors = crate::colors::ColorScheme::terminal_default();
        draw_painted_screen(&refs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), true, &colors, &[], 0);
        // Every cell of the bar (cols 0..5) is REVERSED — including the gap at col 2.
        for x in 0..5u16 {
            assert!(
                buf.cell((x, 1)).unwrap().modifier.contains(Modifier::REVERSED),
                "col {x} of the reverse selection bar is reversed (gap included)"
            );
        }
        // SQ-0490: when the selection moves away the game repaints the row's gaps
        // as PLAIN spaces. Those must stamp too (painter semantics) — repainting
        // over the earlier reversed cells — or the old bar's gap cells stay
        // reversed forever. Same runs re-painted plain, in the same buffer:
        let plain: Vec<crate::engine::PxText> = runs
            .iter()
            .map(|t| crate::engine::PxText { style: 0, ..t.clone() })
            .collect();
        let prefs: Vec<&crate::engine::PxText> = plain.iter().collect();
        draw_painted_screen(&prefs, 0..u16::MAX, 0, area, &mut buf, ratatui::style::Style::default(), true, &colors, &[], 0);
        assert!(
            !buf.cell((2, 1)).unwrap().modifier.contains(Modifier::REVERSED),
            "the gap cell is repainted plain once the selection moves away (SQ-0490)"
        );
    }

    // ── v6 run → cell Style resolution (SQ-0488) ────────────────────────────────

    #[test]
    fn v6_run_style_explicit_standard_sets_channel() {
        // A run with an explicit Standard-palette fg resolves that channel to the
        // palette colour (Zork0's compass letters carry Standard colours), leaving
        // the themed bg intact. Standard(3) is a real palette choice.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let fg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3));
        let s = v6_run_style(base, fg, 0, 0, true, &colors);
        assert_eq!(s.fg, Some(crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(3), &colors)));
        assert_eq!(s.bg, base.bg, "unset bg keeps the theme background");
    }

    #[test]
    fn v6_run_style_true_colour_sets_rgb() {
        // A True24 (24-bit) run resolves to the exact RGB.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x40_2010));
        let s = v6_run_style(base, 0, bg, 0, true, &colors);
        assert_eq!(s.bg, Some(ratatui::style::Color::Rgb(0x40, 0x20, 0x10)));
        assert_eq!(s.fg, base.fg, "unset fg keeps the theme foreground");
    }

    #[test]
    fn v6_run_style_unset_and_default_sentinels_keep_theme() {
        // Default (0) and Standard 0/1 ("current"/"default") are inheritance, not
        // choices — every channel keeps the theme base. Shogun sets no colours, so
        // its Default/Default runs must land here.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        for packed in [
            0u32,
            crate::state::pack_zcolour(zvm::screen::ZColour::Standard(0)),
            crate::state::pack_zcolour(zvm::screen::ZColour::Standard(1)),
        ] {
            let s = v6_run_style(base, packed, packed, 0, true, &colors);
            assert_eq!(s.fg, base.fg, "sentinel {packed:#x} keeps theme fg");
            assert_eq!(s.bg, base.bg, "sentinel {packed:#x} keeps theme bg");
            assert!(!s.add_modifier.contains(ratatui::style::Modifier::REVERSED));
        }
    }

    #[test]
    fn v6_run_style_reverse_bit_toggles_modifier() {
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let rev = v6_run_style(base, 0, 0, 1, true, &colors);
        assert!(rev.add_modifier.contains(ratatui::style::Modifier::REVERSED), "reverse bit adds REVERSED");
        let plain = v6_run_style(base.add_modifier(ratatui::style::Modifier::REVERSED), 0, 0, 0, true, &colors);
        assert!(plain.sub_modifier.contains(ratatui::style::Modifier::REVERSED), "no reverse bit removes REVERSED");
    }

    #[test]
    fn v6_run_style_carries_bold_and_italic() {
        // ZMSD §8.7.1 styles: bit 2 = Bold, bit 4 = Italic (bit 1 = Reverse
        // Video, bit 8 = Fixed Pitch). The v6 cell paths used to drop bold and
        // italic entirely, rendering emphasised menu text as roman.
        use ratatui::style::Modifier;
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        assert!(v6_run_style(base, 0, 0, 2, true, &colors).add_modifier.contains(Modifier::BOLD));
        assert!(v6_run_style(base, 0, 0, 4, true, &colors).add_modifier.contains(Modifier::ITALIC));
        // Combined with reverse video, and unaffected by the colour gate.
        let all = v6_run_style(base, 0, 0, 1 | 2 | 4, false, &colors).add_modifier;
        assert!(all.contains(Modifier::BOLD) && all.contains(Modifier::ITALIC) && all.contains(Modifier::REVERSED));
        // Fixed-pitch (8) alone still adds nothing in a monospaced terminal.
        let fixed = v6_run_style(base, 0, 0, 8, true, &colors);
        assert!(!fixed.add_modifier.contains(Modifier::BOLD) && !fixed.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn v6_run_style_colours_off_returns_theme_base() {
        // honor=false ⇒ explicit colours are ignored, matching every other engine's
        // honor_game_colours gate (Glulx cell_style, the v1-5 grid).
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let fg = crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3));
        let bg = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x123456));
        let s = v6_run_style(base, fg, bg, 0, false, &colors);
        assert_eq!(s.fg, base.fg);
        assert_eq!(s.bg, base.bg);
    }

    #[test]
    fn v6_painted_screen_explicit_run_paints_game_colour() {
        // End-to-end through draw_painted_screen: an explicit Standard-3 fg run
        // stamps with the palette colour, while a Default/Default run keeps the
        // theme base (Shogun's regression pin — its runs are all Default/Default).
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let coloured = crate::engine::PxText {
            y: 1, x: 1, text: "N".into(), style: 0,
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::Standard(3)), bg: 0,
        };
        let plain = crate::engine::PxText { y: 1, x: 25, text: "X".into(), style: 0, fg: 0, bg: 0 };
        let refs: Vec<&crate::engine::PxText> = vec![&coloured, &plain];
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        draw_painted_screen(&refs, 0..u16::MAX, 0, area, &mut buf, base, true, &colors, &[], 0);
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(3), &colors),
            "explicit game colour reaches the buffer cell"
        );
        // The plain run at col 3 (x=25 → (25-1)/8 = 3) keeps the theme fg.
        assert_eq!(buf.cell((3, 0)).unwrap().fg, base.fg.unwrap_or(ratatui::style::Color::Reset), "Default run stays theme-styled");
    }

    #[test]
    fn v6_anchored_band_honours_explicit_run_colour() {
        // The frameless status band resolves an explicit run's colour for its row
        // (Zork0's ribbon labels), while Shogun's Default/Default band keeps theme.
        let colors = crate::colors::ColorScheme::terminal_default();
        let base = colors.theme.get("upper_window").style;
        let coloured = crate::engine::PxText {
            y: 1, x: 1, text: "West of House".into(), style: 0,
            fg: crate::state::pack_zcolour(zvm::screen::ZColour::Standard(4)), bg: 0,
        };
        let refs: Vec<&crate::engine::PxText> = vec![&coloured];
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        draw_anchored_status_band(&refs, 40, 4, area, &mut buf, base, true, &colors);
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(4), &colors),
            "band row adopts the explicit run colour"
        );
        // Shogun regression: a Default/Default run yields exactly the theme fg.
        let plain = crate::engine::PxText { y: 1, x: 1, text: "Shogun".into(), style: 0, fg: 0, bg: 0 };
        let prefs: Vec<&crate::engine::PxText> = vec![&plain];
        let mut buf2 = Buffer::empty(area);
        draw_anchored_status_band(&prefs, 40, 4, area, &mut buf2, base, true, &colors);
        assert_eq!(buf2.cell((0, 0)).unwrap().fg, base.fg.unwrap_or(ratatui::style::Color::Reset), "Default band stays theme-styled");
    }

    /// TEMP measurement harness (SQ-0469). Times the three raster phases —
    /// canvas BUILD (chrome + wrap + glyph blit), content HASH, and
    /// RESIZE+ENCODE — for a real v6 story at a large pane. Run with:
    ///   cargo test -p app bench_v6_raster_phases -- --ignored --nocapture
    #[test]
    #[ignore]
    fn bench_v6_raster_phases() {
        use crate::engine::Engine;
        use std::hash::{Hash, Hasher};
        use std::time::Instant;
        let fg = image::Rgba([220u8, 220, 220, 255]);
        let bg = image::Rgba([0u8, 0, 0, 255]);
        for path in ["stories/zork0-r393-s890714.z6", "stories/shogun-r322-s890706.z6"] {
            let full = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../")
                .join(path);
            let Ok(bytes) = std::fs::read(&full) else {
                println!("SKIP {path}: not found");
                continue;
            };
            let mut sess = match crate::session::GameSession::new(bytes, true, false, None) {
                Ok(s) => s,
                Err(e) => {
                    println!("SKIP {path}: {e:?}");
                    continue;
                }
            };
            for cmd in ["look", "open mailbox", "look"] {
                let _ = sess.submit(cmd);
            }
            let model = sess.screen();
            let items = match &model.root {
                WinNode::Layered(v) => v.clone(),
                other => {
                    println!("SKIP {path}: root is {other:?}, not Layered");
                    continue;
                }
            };
            let native = crate::render::v6_layout::native_extent(&items);

            let mut state = AppState::default();
            state.colors = crate::colors::ColorScheme::terminal_default();
            state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
            for i in 0..2000 {
                state
                    .transcript
                    .push(format!("The quick brown fox line {i} jumps over the lazy dog by the white house door."));
            }
            state.transcript_styles.resize(state.transcript.len(), None);
            state.transcript_runs.resize(state.transcript.len(), Vec::new());
            state.transcript_para.resize(state.transcript.len(), crate::state::ParaFmt::default());
            state.transcript_images.resize(state.transcript.len(), None);

            // A large pane: 220x64 cells at halfblocks 10x20 px = 2200x1280 device.
            let picker = state.game_picker.clone().unwrap();
            let area = Rect::new(0, 0, 220, 64);

            // Build closure: replicate the raster branch's canvas construction.
            let build = || {
                let layout = crate::render::v6_layout::classify_windows(&items);
                let mut canvas = crate::render::v6_layout::build_chrome_canvas(&layout.chrome, native, fg, bg, &state.colors);
                if let Some((sx, sy, sw, sh)) = crate::render::v6_layout::story_clear_native(layout.story, &canvas) {
                    let cols = (sw / 8).max(1) as u16;
                    let rows = (sh / 8).max(1) as u16;
                    let (main, _) = build_main_text(&state, cols, rows);
                    crate::render::v6_layout::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, fg, &[]);
                }
                canvas
            };

            const N: u32 = 30;
            // Phase GEN (SQ-0469): the whole cost of an idle/unchanged frame after
            // the gate — no build, no hash, no encode.
            let t = Instant::now();
            let mut gsum = 0u64;
            for _ in 0..N {
                gsum ^= v6_raster_gen(&items, &state, area, &picker);
            }
            let gen_us = t.elapsed().as_micros() as f64 / N as f64;
            std::hint::black_box(gsum);

            // Phase BUILD.
            let t = Instant::now();
            let mut canvas = build();
            for _ in 1..N {
                canvas = build();
            }
            let build_us = t.elapsed().as_micros() as f64 / N as f64;

            // Phase HASH.
            let t = Instant::now();
            let mut hsum = 0u64;
            for _ in 0..N {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                canvas.as_raw().hash(&mut h);
                hsum ^= h.finish();
            }
            let hash_us = t.elapsed().as_micros() as f64 / N as f64;
            std::hint::black_box(hsum);

            // Phase RESIZE+ENCODE (uncapped, as shipped).
            let fs = picker.font_size();
            let box_w = area.width as u32 * fs.width.max(1) as u32;
            let box_h = area.height as u32 * fs.height.max(1) as u32;
            let (cw, ch) = (canvas.width(), canvas.height());
            let encode = |cap: f64| {
                let scale = ((box_w as f64 / cw as f64).min(box_h as f64 / ch as f64)).max(1.0).min(cap);
                let (tw, th) = ((cw as f64 * scale) as u32, (ch as f64 * scale) as u32);
                let scaled = image::imageops::resize(&canvas, tw.max(cw), th.max(ch), image::imageops::FilterType::Nearest);
                let img = image::DynamicImage::ImageRgba8(scaled);
                let _ = picker.new_protocol(img, ratatui::layout::Size::new(area.width, area.height), ratatui_image::Resize::Fit(None));
            };
            let t = Instant::now();
            for _ in 0..N {
                encode(f64::INFINITY);
            }
            let enc_us = t.elapsed().as_micros() as f64 / N as f64;
            let t = Instant::now();
            for _ in 0..N {
                encode(4.0);
            }
            let enc4_us = t.elapsed().as_micros() as f64 / N as f64;

            println!(
                "\n=== {path} ===\n native canvas: {}x{}  pane device: {}x{}\n GEN (idle key):   {gen_us:>9.1} us/frame\n BUILD:            {build_us:>9.1} us/frame\n HASH:             {hash_us:>9.1} us/frame\n ENCODE (uncap):   {enc_us:>9.1} us/frame\n ENCODE (cap 4x):  {enc4_us:>9.1} us/frame\n --- BEFORE (no gate; build+hash every frame) ---\n IDLE / keystroke frame  = {:.1} us (build+hash on main)\n CHANGED frame           = {:.1} us (build+hash+encode on main)\n --- AFTER (SQ-0469 gate + cap + worker) ---\n IDLE frame              = {gen_us:.1} us (gen key only)\n KEYSTROKE/CHANGED frame = {:.1} us on main (gen+build; capped encode {enc4_us:.1} us OFF-thread)",
                native.0, native.1, box_w, box_h,
                build_us + hash_us,
                build_us + hash_us + enc_us,
                gen_us + build_us,
            );
        }
    }
}
