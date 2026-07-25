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
        return StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: tarea.height, total_rows, links };
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

    let mut m = metrics.unwrap_or(StoryPaneMetrics { scrollbar: false, max_scroll: 0, viewport_rows: area.height, total_rows: 0, links: Vec::new() });
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
    // Effective margin: a discovered garglk.ini's tmarginx/tmarginy wins (highest
    // precedence, runtime-only — never persisted), else the global config default
    // (SQ-0344).
    let ov = state.garglk_overlay.as_ref();
    let want_x = ov.and_then(|o| o.margin_x).unwrap_or(state.config.text_margin_x);
    let want_y = ov.and_then(|o| o.margin_y).unwrap_or(state.config.text_margin_y);
    let mx = want_x.min(area.width.saturating_sub(1) / 2);
    let my = want_y.min(area.height.saturating_sub(1) / 2);
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
                Some(StoryPaneMetrics { scrollbar, max_scroll, viewport_rows: area.height, total_rows, links })
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
                fill(area, buf, &state.colors);
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
            let story_rows = items.iter().find_map(|pw| {
                matches!(&pw.node, WinNode::Buffer(_)).then(|| {
                    let top = pw.y_px / 16;
                    (top, top + pw.h_px.max(1).div_ceil(16))
                })
            });
            let has_menu = items.iter().any(|pw| {
                matches!(&pw.node, WinNode::Grid(g)
                    if g.px_texts.iter().any(|t| {
                        let row = (t.y.max(1) - 1) / 16;
                        !t.text.trim().is_empty()
                            && row >= STATUS_BAND_ROWS
                            && story_rows.is_some_and(|(top, bot)| row >= top && row < bot)
                    }))
            });
            if !state.any_overlay_open() && !frameless && !(has_menu && hybrid) {
            if let Some(picker) = state.game_picker.as_ref() {
                let theme_bg = state.colors.theme.get("transcript").style;
                // SQ-0510: a themed RGB wins; else the terminal's own default
                // (OSC 10/11, probed at startup); else the hardcoded fallback.
                let default_fg = v6_default_fg(theme_bg, state.term_default_colors.fg);
                let default_bg = v6_default_bg(theme_bg, state.term_default_colors.bg);
                use crate::render::v6_layout as v6;
                let native = v6::native_extent(items);
                let layout = v6::classify_windows(items);
                // The native chrome canvas is built per-branch below (SQ-0469):
                // the raster arm skips the build entirely on an unchanged frame.

                // Hybrid mode (Lane H): draw the chrome as a scaled pixel RING
                // around a terminal story viewport, then render the story window as
                // real terminal text (crisp, selectable, scrollable) inside it — the
                // existing primary-Buffer transcript path, with inline images as
                // bands. Needs a story window; without one, fall through to raster.
                if state.config.v6_render == crate::config::V6RenderMode::Hybrid {
                    if let Some(story) = layout.story {
                        let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &state.colors);
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
                            BottomPlan::Extend => {
                                let vp = v6::story_viewport_box(Some(story), &top_scale, (area.width, area.height), cell_px);
                                let (x, y) = (area.x + vp.x, area.y + vp.y);
                                (top_scale, Rect::new(x, y, vp.width, area.bottom().saturating_sub(y)), None)
                            }
                            BottomPlan::Menu => {
                                let menu_scale = v6::Scale { s: scale_center.s, off_x: scale_center.off_x, off_y: slack };
                                let vp = v6::story_viewport_box(Some(story), &top_scale, (area.width, area.height), cell_px);
                                let (x, y) = (area.x + vp.x, area.y + vp.y);
                                // The menu strip's top cell: the story's native bottom
                                // mapped through the bottom-anchored menu scale (whose
                                // native bottom lands exactly on the pane bottom).
                                let story_bottom = story.y_px as u32 + story.h_px as u32;
                                let menu_top_dev = slack as f32 + story_bottom as f32 * scale_center.s;
                                let menu_top = (menu_top_dev / cell_px.1.max(1) as f32).floor() as u16;
                                let menu_top = menu_top.clamp(y + 1, area.bottom());
                                (top_scale, Rect::new(x, y, vp.width, menu_top.saturating_sub(y)), Some(menu_scale))
                            }
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
                        // SQ-0505: when reclaiming the dead space, clip the ring
                        // bands to the chrome art's actual vertical extent (its
                        // lowest opaque native row, mapped through the story scale).
                        // The flanks BELOW the side art then stay the theme backdrop
                        // rather than an opaque transparent-crop block — no art
                        // stretching or tiling into the reclaimed space. Letterbox is
                        // untouched (its bands lie within the scaled canvas anyway).
                        if reclaim {
                            let ch = cell_px.1.max(1) as f32;
                            let art_bottom_px =
                                (0..gfx.height()).rev().find(|&y| (0..gfx.width()).any(|x| gfx.get_pixel(x, y)[3] >= 128));
                            let clip_row = match art_bottom_px {
                                Some(y) => area.y + ((scale.off_y as f32 + (y + 1) as f32 * scale.s) / ch).ceil() as u16,
                                None => area.y,
                            };
                            for b in &mut ring_bands {
                                if b.y >= clip_row {
                                    b.height = 0;
                                } else {
                                    b.height = b.height.min(clip_row - b.y);
                                }
                            }
                            ring_bands.retain(|b| b.height > 0 && b.width > 0);
                        }
                        let strips = decompose_chrome_strips(&ring_bands, area, &scale, cell_px, story, &gfx, &chrome_runs);
                        let menu_strips = match &menu {
                            Some(ms) => decompose_chrome_strips(&menu_bands, area, ms, cell_px, story, &gfx, &chrome_runs),
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
                        {
                            let mut gr = state.graphics_render.borrow_mut();
                            let live: std::collections::HashSet<_> = strips
                                .iter()
                                .chain(menu_strips.iter())
                                .filter_map(|s| match s {
                                    ChromeStrip::Art(r) => Some((r.x, r.y, r.width, r.height)),
                                    ChromeStrip::Text(..) => None,
                                })
                                .collect();
                            gr.retain_chrome_bands(&live);
                            for strip in &strips {
                                match strip {
                                    ChromeStrip::Art(r) => gr.draw_chrome_band(picker, &canvas, &scale, area, *r, buf),
                                    ChromeStrip::Text(r, runs) => draw_chrome_text_strip(
                                        runs, *r, &scale, cell_px, area, base, state.config.honor_game_colours, &state.colors, buf,
                                    ),
                                }
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
                            gr.record_hybrid_click_map(area, click_scale, native, cell_px);
                        }
                        // The story window as real terminal text (primary-Buffer path).
                        let metrics = render_node(&story.node, status, char_mode, introspect, state, viewport, buf, game_input, links, grid_colors);
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
                                        buf.set_stringn(col as u16, row as u16, &t.text, max_w, style);
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
                    if runs.iter().any(|t| !t.text.trim().is_empty()) {
                        draw_painted_screen(&runs, 0, area, buf, status_style, state.config.honor_game_colours, &state.colors);
                        return None;
                    }
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
                if state.graphics_render.borrow().v6_wants_build(gen, area) {
                    let mut canvas = v6::build_chrome_canvas(&layout.chrome, native, default_fg, default_bg, &state.colors);
                    let mut raster_metrics: Option<RasterMetrics> = None;
                    if let Some((sx, sy, sw, sh)) = v6::story_clear_native(layout.story, &canvas) {
                        // The story window's own background colour (set by the game
                        // via set_colour), when it set one — paints the page instead
                        // of leaving it transparent over the theme backdrop. No
                        // colour set ⇒ unchanged (transparent) behaviour.
                        if let Some(color) = v6::story_bg_rgba(layout.story, &state.colors) {
                            v6::fill_cell(&mut canvas, sx, sy, sw, sh, color);
                        }
                        // Window-0 inline pictures (drop-caps, room icons) arrive as
                        // transcript-anchored floats (`transcript_images` sidecar):
                        // build_main_text wraps text beside them and draw_story_text
                        // blits each at its anchored row — they scroll with the text.
                        // Non-square 8×16 v6 cell (SQ-0479): columns divide the
                        // clear width by FONT_W(8), rows the height by FONT_H(16).
                        let cols = (sw / 8).max(1) as u16;
                        let rows = (sh / 16).max(1) as u16;
                        let (main, rm) = build_main_text(state, cols, rows);
                        v6::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, default_fg);
                        // [more] pager indicator (SQ-0455): when a single turn's output
                        // overflowed the story box the shared pager (SQ-0404) parks the
                        // scroll and shows a `[more]` prompt. The raster path can't reserve
                        // a terminal row, so draw the prompt as a text run bottom-right of
                        // the story box, themed via the `more_prompt` selector (drawn as a
                        // reverse-video block, matching the terminal bar).
                        if state.pager.active {
                            let mp = state.colors.theme.get("more_prompt").style;
                            // Reverse-video: block = default ink, text = default
                            // page; unthemed selectors fall to the OSC/hard defaults.
                            let block = v6_default_fg(mp, state.term_default_colors.fg);
                            let ink = v6_default_bg(mp, state.term_default_colors.bg);
                            let label = "[more]";
                            let n = label.chars().count() as u32;
                            let last_row = rows.saturating_sub(1) as u32;
                            let start_col = (cols as u32).saturating_sub(n);
                            for (i, ch) in label.chars().enumerate() {
                                // 8×16 cell: X by FONT_W(8), Y by FONT_H(16).
                                crate::render::bitfont::blit_glyph(
                                    &mut canvas, ch, sx + (start_col + i as u32) * 8, sy + last_row * 16, 8, 16, ink, Some(block),
                                );
                            }
                        }
                        raster_metrics = Some(rm);
                    }
                    // Cache the fresh metrics for skipped frames, then hand the
                    // built canvas to the off-thread resize+encode worker.
                    state.v6_raster_metrics.set(raster_metrics);
                    state.graphics_render.borrow_mut().spawn_v6_encode(picker, canvas, gen, area);
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
                let (native_w, _) = crate::render::v6_layout::native_extent(items);
                let ncols = (native_w as u32).div_ceil(8).max(1);
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
                    let rows_used = draw_anchored_status_band(&runs, ncols, area, buf, status_style, state.config.honor_game_colours, &state.colors);
                    let story_area = Rect::new(
                        area.x,
                        area.y + rows_used,
                        area.width,
                        area.height.saturating_sub(rows_used),
                    );
                    let m = render_node(&story.node, status, char_mode, introspect, state, story_area, buf, game_input, links, grid_colors);
                    // Painted-screen overlay (SQ-0478): stamp any DEEP paint runs
                    // (native row ≥ 4, below the status band) as absolutely-
                    // positioned terminal text on TOP of the story transcript.
                    // A no-op in normal gameplay (chrome grids carry only the
                    // top status runs); on a menu screen it draws the items +
                    // the reverse-video selection caret the anchored band drops.
                    draw_painted_screen(&runs, STATUS_BAND_ROWS, area, buf, status_style, state.config.honor_game_colours, &state.colors);
                    return m;
                }
                // No streaming story window (a painted menu with win0 in paint
                // mode, or none open): the whole pane IS a painted text screen —
                // stamp every run absolutely rather than falling through to the
                // z-ordered cell composite, which renders the native geometry as
                // an unreadable postage stamp (SQ-0478).
                if runs.iter().any(|t| !t.text.trim().is_empty()) {
                    draw_painted_screen(&runs, 0, area, buf, status_style, state.config.honor_game_colours, &state.colors);
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

/// Resolve the v6 raster canvas's default ink (SQ-0510). Layering: an explicitly
/// themed RGB fg wins; else the terminal's OSC-10 default (when it answered);
/// else the hardcoded light-grey fallback.
fn v6_default_fg(themed: ratatui::style::Style, osc: Option<image::Rgba<u8>>) -> image::Rgba<u8> {
    style_fg_rgba(themed, osc.unwrap_or(RASTER_FALLBACK_INK))
}

/// Resolve the v6 raster canvas's default page (SQ-0510). Layering: an explicitly
/// themed RGB bg wins; else the terminal's OSC-11 default (when it answered);
/// else the hardcoded black fallback.
fn v6_default_bg(themed: ratatui::style::Style, osc: Option<image::Rgba<u8>>) -> image::Rgba<u8> {
    style_bg_rgba(themed, osc.unwrap_or(RASTER_FALLBACK_PAGE))
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
    let mut wrapped: Vec<String> = Vec::new();
    let mut floats: Vec<AbsFloat> = Vec::new();
    for (i, line) in state.transcript.iter().enumerate() {
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
        // Word-wrap with per-row width: rows beside an active float are narrower.
        let mut cur = String::new();
        for word in line.split(' ') {
            let width = cols.saturating_sub(reserve_at(&floats, wrapped.len())).max(1) as usize;
            if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > width {
                wrapped.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
        wrapped.push(cur);
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
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(budget);
    let visible_len = end - start;
    let lines = wrapped[start..end].to_vec();
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
    // Show the input line + caret only when the game has host focus AND the view
    // is at the bottom — scrolled-back history must not be overwritten by the live
    // line (matching the terminal transcript's `effective_scroll == 0` guard).
    let awaiting = scroll == 0 && matches!(state.focus, crate::state::Focus::Game);
    let main = crate::render::v6_layout::MainText { lines, input, cursor_col, awaiting, floats };
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

/// The number of native cell rows the top status band owns (rows 0..N). Paint
/// runs at or below this row are the DEEP menu/hint content the painted-screen
/// overlay handles; rows above it are the anchored status line. (SQ-0478)
const STATUS_BAND_ROWS: u16 = 4;

/// Render a v6 PAINTED text screen (menus, hints — SQ-0477/0478) as absolutely-
/// positioned terminal text. Each run is quantized to its native cell
/// (`col = (x-1)/8`, `row = (y-1)/16` — the non-square 8×16 v6 cell) and stamped at that pane-relative cell,
/// honoring reverse video — Shogun's boot-menu selection is a reverse-video run,
/// so this is what makes the selection caret visible. Menus are absolutely
/// positioned (NOT left/center/right anchor groups like the status band).
///
/// Only runs at native `row >= min_row` are drawn: the frameless path overlays
/// the DEEP runs (`min_row = STATUS_BAND_ROWS`) above the anchored status band
/// which owns the top rows, while a story-less menu screen passes `min_row = 0`
/// to stamp the whole pane. Shared by the frameless and hybrid (no story window)
/// paths so both present a painted screen identically.
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
    if style_bits & 1 != 0 {
        s.add_modifier(ratatui::style::Modifier::REVERSED)
    } else {
        s.remove_modifier(ratatui::style::Modifier::REVERSED)
    }
}

fn draw_painted_screen(
    runs: &[&crate::engine::PxText],
    min_row: u16,
    area: Rect,
    buf: &mut Buffer,
    base: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
) {
    for t in runs {
        // Every run stamps, whitespace included — painter semantics. A reversed
        // space fills its cell of the selection bar (SQ-0484), and a NORMAL
        // space must equally repaint over an earlier reversed one: when the
        // menu selection moves, the game repaints the old row's gaps as plain
        // spaces, and skipping those left the stale reversed cells behind
        // (SQ-0490).
        // 8×16 v6 cell (SQ-0479): quantize Y by FONT_H(16), X by FONT_W(8).
        let row = (t.y.max(1) - 1) / 16;
        if row < min_row {
            continue;
        }
        let col = (t.x.max(1) - 1) / 8;
        if area.y + row >= area.bottom() || area.x + col >= area.right() {
            continue;
        }
        // Cell styles PATCH — a repaint must explicitly clear the reverse bit,
        // or a cell once reversed stays reversed after the game repaints it
        // plain (SQ-0490). Explicit game colours on the run replace the theme
        // base per channel; inherited/Default channels keep it (SQ-0488).
        let style = v6_run_style(base, t.fg, t.bg, t.style, honor, colors);
        let max_w = (area.right() - (area.x + col)) as usize;
        buf.set_stringn(area.x + col, area.y + row, &t.text, max_w, style);
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
enum BottomPlan {
    Letterbox,
    Extend,
    Menu,
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
    // Story fills to (within one native row of) the screen bottom → enclosed
    // frame, nothing to reclaim in-frame; keep the centred letterbox.
    if native.1 as u32 <= story_bottom + 16 {
        return BottomPlan::Letterbox;
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

/// Map a chrome run's native top-left game pixel to its pane-absolute terminal
/// cell (col, row) through the letterbox `scale` — the same mapping the pixel
/// ring and the inside-story overlay use, so a text strip lines up exactly with
/// the art strips beside it.
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
fn decompose_chrome_strips<'a>(
    bands: &[Rect],
    pane: Rect,
    scale: &crate::render::v6_layout::Scale,
    cell_px: (u16, u16),
    story: &crate::engine::PositionedWindow,
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
    let below_or_above = |t: &crate::engine::PxText| -> bool {
        let py = t.y.max(1) as i32 - 1;
        py >= story_bottom || py + 16 <= story_top
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
    }
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
            let above = (0..i).rev().find(|&j| !matches!(classes[j], RowClass::Empty));
            let below = (i + 1..n).find(|&j| !matches!(classes[j], RowClass::Empty));
            if above.is_some_and(|j| is_text(&classes[j])) && below.is_some_and(|j| is_text(&classes[j])) {
                bridge[i] = true;
            }
        }
        // Coalesce consecutive same-class (Text|bridged vs. not) rows into strips.
        let mut i = 0usize;
        while i < n {
            let text = matches!(classes[i], RowClass::Text(_)) || bridge[i];
            let mut j = i;
            let mut text_runs: Vec<&crate::engine::PxText> = Vec::new();
            while j < n && (matches!(classes[j], RowClass::Text(_)) || bridge[j]) == text {
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

    // Bucket runs by their scale-mapped terminal row.
    let mut raw: BTreeMap<i32, Vec<&PxText>> = BTreeMap::new();
    for t in runs {
        let (_, row) = run_cell(t, scale, cell_px, pane);
        raw.entry(row).or_default().push(t);
    }
    // SQ-0509: merge horizontally-contiguous same-style fragments before mapping.
    // A game (Arthur) that positions status text with proportional pixel metrics
    // emits word fragments as separate runs whose pixel start abuts the previous
    // run's pixel end; mapping each fragment independently through the letterbox
    // scale rounds them into stray cell gaps ("Chu rch yard"). Runs within
    // `FRAG_TOL` px of the previous end (and identical style/colours) concatenate
    // into one run stamped contiguously from a single mapped cell; runs separated
    // by a genuine gap (Journey's menu items / column dividers, 8px apart) stay
    // distinct and keep their proportional spacing.
    const FRAG_TOL: i32 = 4;
    let mut by_row: BTreeMap<i32, Vec<PxText>> = BTreeMap::new();
    for (row, mut rr) in raw {
        rr.sort_by_key(|t| t.x);
        let mut merged: Vec<PxText> = Vec::new();
        for t in rr {
            if let Some(last) = merged.last_mut() {
                let last_end = (last.x.max(1) as i32 - 1) + last.text.chars().count() as i32 * 8;
                let start = t.x.max(1) as i32 - 1;
                if start >= last_end
                    && start - last_end <= FRAG_TOL
                    && last.style == t.style
                    && last.fg == t.fg
                    && last.bg == t.bg
                {
                    last.text.push_str(&t.text);
                    continue;
                }
            }
            merged.push((*t).clone());
        }
        by_row.insert(row, merged);
    }

    // SQ-0508(b): divider columns to draw continuously. A reversed WHITESPACE run in
    // a MIXED row (normal verb text among reversed dividers — Journey's menu body) is
    // a vertical column divider; extend every such column across the FULL strip height
    // so the scale-introduced gap rows (bridged in as blank Text rows) don't break the
    // lines. Collected from mixed rows only, so a pure-reverse bar row (a header /
    // status bar, already filled edge to edge below) contributes none.
    let mut divider_cols: Vec<u16> = Vec::new();
    for row_runs in by_row.values() {
        let mixed = row_runs.iter().any(|t| t.style & 1 == 0);
        if !mixed {
            continue;
        }
        for t in row_runs {
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
        let all_rev = !row_runs.is_empty() && row_runs.iter().all(|t| t.style & 1 != 0);
        let row_fg = row_runs.iter().map(|t| t.fg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        let row_bg = row_runs.iter().map(|t| t.bg).find(|&p| crate::render::v6_layout::packed_explicit(p)).unwrap_or(0);
        if all_rev
            || crate::render::v6_layout::packed_explicit(row_fg)
            || crate::render::v6_layout::packed_explicit(row_bg)
        {
            let fill = v6_run_style(base, row_fg, row_bg, all_rev as u8, honor, colors);
            for c in rect.x..rect.right() {
                buf.set_stringn(c, *row as u16, " ", 1, fill);
            }
        }
        for t in row_runs {
            let (col, _) = run_cell(t, scale, cell_px, pane);
            if col < rect.x as i32 || col >= rect.right() as i32 {
                continue;
            }
            let style = v6_run_style(base, t.fg, t.bg, t.style, honor, colors);
            let max_w = rect.right() as usize - col as usize;
            if max_w > 0 {
                buf.set_stringn(col as u16, *row as u16, &t.text, max_w, style);
            }
        }
    }
}

/// Render the v6 frameless status band as a classic full-width status line
/// ("anchored bar", SQ-0467). `runs` are all the chrome grids' pixel-text runs;
/// `ncols` is the native screen width in cells (so anchor thresholds scale to the
/// game's own screen, not a hardcoded 40). Each native row (`(y-1)/16`, capped at
/// 4) is classified into LEFT/CENTER/RIGHT anchor groups and painted across the
/// full pane width. Returns the number of band rows used (for the story offset).
fn draw_anchored_status_band(
    runs: &[&crate::engine::PxText],
    ncols: u32,
    area: Rect,
    buf: &mut Buffer,
    style: ratatui::style::Style,
    honor: bool,
    colors: &ColorScheme,
) -> u16 {
    let left_bound = ncols / 3; // left-third boundary (cells)
    let right_bound = ncols * 2 / 3; // right two-thirds boundary (cells)
    let mut rows_used = 0u16;
    for row in 0..4u16 {
        if area.y + row >= area.bottom() {
            break; // status band is at most 4 rows and must stay in-pane
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
        // Classify each run into an anchor group by its native position. A run
        // spanning most of the row (a full-width bar) counts LEFT; otherwise a
        // start in the left third is LEFT, an end past the right two-thirds is
        // RIGHT, and everything between is CENTER. Within a group, run order is
        // preserved and the native gaps collapse to a two-space join.
        let (mut left, mut center, mut right): (Vec<&str>, Vec<&str>, Vec<&str>) =
            (Vec::new(), Vec::new(), Vec::new());
        for t in &row_runs {
            let start = ((t.x.max(1) - 1) / 8) as u32;
            let len = t.text.chars().count() as u32;
            let end = start + len;
            if len * 3 >= ncols * 2 || start < left_bound {
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
        if place_anchored_row(buf, area, area.y + row, &left_str, &center_str, &right_str, row_style) {
            rows_used = rows_used.max(row + 1);
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

    #[test]
    fn v6_default_colours_layer_theme_over_osc_over_fallback() {
        use ratatui::style::{Color, Style};
        let osc_fg = Some(image::Rgba([10, 20, 30, 255]));
        let osc_bg = Some(image::Rgba([40, 50, 60, 255]));

        // Theme sets a concrete RGB → it wins over the OSC probe.
        let themed = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Rgb(4, 5, 6));
        assert_eq!(v6_default_fg(themed, osc_fg), image::Rgba([1, 2, 3, 255]));
        assert_eq!(v6_default_bg(themed, osc_bg), image::Rgba([4, 5, 6, 255]));

        // Theme leaves fg/bg at "terminal default" (no concrete RGB) → OSC fills in.
        let unset = Style::default();
        assert_eq!(v6_default_fg(unset, osc_fg), image::Rgba([10, 20, 30, 255]));
        assert_eq!(v6_default_bg(unset, osc_bg), image::Rgba([40, 50, 60, 255]));

        // No theme AND no OSC answer → today's hardcoded fallbacks are preserved.
        assert_eq!(v6_default_fg(unset, None), RASTER_FALLBACK_INK);
        assert_eq!(v6_default_bg(unset, None), RASTER_FALLBACK_PAGE);
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
            WinNode::Buffer(BufferWindow { lines: vec![], runs: vec![], para: vec![], images: vec![], scroll: 0, primary, bg: Some(bg), fg: None, panel: false })
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
        // All three items render as terminal text on their distinct deep rows.
        assert_eq!(row_text(8).trim(), "START the game", "row 8 is the START item, screen:\n{screen}");
        assert_eq!(row_text(9).trim(), "RESTORE a saved game", "row 9 is the RESTORE item");
        assert_eq!(row_text(10).trim(), "QUIT the game", "row 10 is the QUIT item");
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
    /// primary story `Buffer`. No decorative graphics window.
    fn frameless_v6_model() -> ScreenModel {
        let status = PositionedWindow {
            x: 0, y: 0, w: 40, h: 1, x_px: 0, y_px: 0, w_px: 320, h_px: 8,
            left_margin: 0, right_margin: 0,
            node: WinNode::Grid(crate::engine::GridWindow {
                cols: 40, rows: 1, cells: vec![], active_rows: 1, cursor: (1, 1),
                cursor_active: false, border: crate::engine::BorderPref::Unspecified,
                bg: None, fg: None, reverse: false,
                px_texts: vec![
                    crate::engine::PxText { y: 1, x: 1, text: "SCORE 10".into(), style: 0, fg: 0, bg: 0 },
                ],
            }),
        };
        let story = PositionedWindow {
            x: 0, y: 1, w: 40, h: 24, x_px: 0, y_px: 8, w_px: 320, h_px: 192,
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
        let rows_used = draw_anchored_status_band(&refs, ncols, area, &mut buf, style, true, &colors);
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

    #[test]
    fn anchored_band_multi_row() {
        // Runs on native rows 0 and 1 (y=1 and y=17, one 16px cell apart) each
        // render on their own band row, and rows_used reports 2.
        let runs = [run(1, 1, "Row0Left"), run(233, 1, "Score: 0"), run(1, 17, "Row1Left")];
        let refs: Vec<&crate::engine::PxText> = runs.iter().collect();
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        let colors = crate::colors::ColorScheme::terminal_default();
        let rows_used = draw_anchored_status_band(&refs, 40, area, &mut buf, ratatui::style::Style::default(), true, &colors);
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
        draw_painted_screen(&refs, 0, area, &mut buf, ratatui::style::Style::default(), true, &colors);
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
        draw_painted_screen(&prefs, 0, area, &mut buf, ratatui::style::Style::default(), true, &colors);
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
        draw_painted_screen(&refs, 0, area, &mut buf, base, true, &colors);
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
        draw_anchored_status_band(&refs, 40, area, &mut buf, base, true, &colors);
        assert_eq!(
            buf.cell((0, 0)).unwrap().fg,
            crate::render::resolve_zcolour(zvm::screen::ZColour::Standard(4), &colors),
            "band row adopts the explicit run colour"
        );
        // Shogun regression: a Default/Default run yields exactly the theme fg.
        let plain = crate::engine::PxText { y: 1, x: 1, text: "Shogun".into(), style: 0, fg: 0, bg: 0 };
        let prefs: Vec<&crate::engine::PxText> = vec![&plain];
        let mut buf2 = Buffer::empty(area);
        draw_anchored_status_band(&prefs, 40, area, &mut buf2, base, true, &colors);
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
                    crate::render::v6_layout::draw_story_text(&mut canvas, &main, sx, sy, cols, rows, fg);
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
