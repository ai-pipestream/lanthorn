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
use crate::engine::{BorderPref, BufferWindow, Introspect, ScreenModel, StatusModel, WinNode};
use crate::render::transcript::{draw_str_runs, render_transcript, visible_wrapped_lines_kinded};
use crate::render::upper_window::{draw_grid, draw_upper_window};
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
    /// Per-frame map from rendered cell `(col, row)` → Glk hyperlink value, for
    /// hit-testing a mouse click to its link. Empty when nothing is linked.
    pub links: Vec<((u16, u16), u32)>,
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
fn game_input_style(model: &ScreenModel, state: &AppState) -> Option<ratatui::style::Style> {
    if !state.config.honor_game_colours {
        return None;
    }
    let fg = crate::state::unpack_zcolour(model.fg);
    let bg = crate::state::unpack_zcolour(model.bg);
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
    let mut base = c.upper_window;
    if !matches!(fg, ZColour::Default) {
        base = base.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        base = base.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    c.upper_window = base;
    // The border chrome is entirely our own presentation — Glk provides no border
    // styling — so paint the frame in the same page colours as the content, making
    // the whole status area (content + border) one coloured block on the recoloured
    // page rather than a themed frame around a game-coloured interior. (SQ-0267)
    let mut border = c.upper_window_border;
    if !matches!(fg, ZColour::Default) {
        border = border.fg(crate::render::resolve_zcolour(fg, &state.colors));
    }
    if !matches!(bg, ZColour::Default) {
        border = border.bg(crate::render::resolve_zcolour(bg, &state.colors));
    }
    c.upper_window_border = border;
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
        let (scrollbar, max_scroll, mut tlinks) = render_transcript(&model.status, introspect, state, tarea, buf, gi);
        links.append(&mut tlinks);
        return StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: tarea.height, links };
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
    state.graphics_render.borrow_mut().retain_live(&live);

    let mut m = metrics.unwrap_or(StoryPaneMetrics { scrollbar: false, max_scroll: 0, viewport_rows: area.height, links: Vec::new() });
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
    state.colors.transcript
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
            // it (SQ-0325).
            if b > 0 && !a1.is_empty() && !a2.is_empty() {
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
                let (scrollbar, max_scroll, links) =
                    render_transcript(status, introspect, state, area, buf, game_input);
                Some(StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: area.height, links })
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
            if let Some(picker) = state.game_picker.as_ref() {
                state.graphics_render.borrow_mut().render(picker, gw, area, state.colors.graphics, buf);
            } else {
                fill(area, buf, &state.colors);
            }
            None
        }
    }
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
    }
}

/// Collect the window ids of all live graphics windows in the tree.
fn collect_graphics_ids(node: &WinNode, out: &mut std::collections::HashSet<u32>) {
    match node {
        WinNode::Graphics(gw) => {
            out.insert(gw.win);
        }
        WinNode::Pair { first, second, .. } => {
            collect_graphics_ids(first, out);
            collect_graphics_ids(second, out);
        }
        _ => {}
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
/// in `colors.upper_window_border` (the same Style the status frame uses), and a
/// user-set glyph override from `colors.upper_window_border_glyphs` — `.top` for a
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
    let mut style = colors.upper_window_border;
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
    let base = match b.bg {
        Some(rgb) => state.colors.transcript.bg(crate::render::resolve_zcolour(zvm::screen::ZColour::True24(rgb), &state.colors)),
        None => state.colors.transcript,
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
    fill_style(area, buf, colors.transcript);
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

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{GridWindow, Split};
    use crate::state::StyleRun;
    use ratatui::layout::Rect;

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
        assert_eq!(gc.upper_window.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)));
        assert_eq!(gc.upper_window.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)));
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
        assert_eq!(gc.upper_window_border.bg, Some(ratatui::style::Color::Rgb(255, 255, 255)),
            "border background matches the game page background");
        assert_eq!(gc.upper_window_border.fg, Some(ratatui::style::Color::Rgb(0, 0, 0)),
            "border line drawn in the game page foreground ink");
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
        }
    }

    fn row_text(buf: &Buffer, y: u16, w: u16) -> String {
        (0..w)
            .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
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

        // Frameless: NO box-drawing glyph anywhere in the pane.
        for y in 0..10 {
            for x in 0..20 {
                let s = buf.cell((x, y)).unwrap().symbol();
                assert!(
                    !"┌┐└┘─│".contains(s),
                    "no frame glyph on the generic path, found {s:?} at ({x},{y})"
                );
            }
        }
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
        assert!(side[9..].contains("SIDE"), "side panel level with grid on row 0: {side:?}");
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
            scaled: None,
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
        // row) carries the game background (black).
        assert_eq!(buf.cell((0, 2)).unwrap().style().bg, Some(Color::Black),
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
        let (sb, ms, _) = render_transcript(&model.status, None, &state, tarea, &mut buf_b, None);

        assert_eq!(buf_a, buf_b, "the simple path must be byte-identical to the legacy path");
        assert_eq!((ma.scrollbar, ma.max_scroll, ma.viewport_rows), (sb, ms, tarea.height));
    }

    fn graphics_node() -> WinNode {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 1,
            canvas: std::sync::Arc::new(img),
            version: 1,
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
        // MapFull: story pane isn't laid out (empty), so graphics aren't on screen
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
        let gw = crate::engine::GraphicsWindow { win: 1, canvas: std::sync::Arc::new(img), version: 1 };
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
            state.colors.upper_window_border.fg,
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
            state.colors.upper_window_border.fg,
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
    fn collect_graphics_ids_finds_every_graphics_leaf() {
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let other = WinNode::Graphics(crate::engine::GraphicsWindow {
            win: 7,
            canvas: std::sync::Arc::new(img),
            version: 1,
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
}
