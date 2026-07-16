//! Story-picker UI subsystem: the pre-game story browser and its metadata
//! info panel. Extracted verbatim from `main.rs` (SQ-0306) as the UI companion
//! to the `app::picker` logic module. Pure move — no behavior change.

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{read, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use app::anim::PanelSlide;
use app::render::draw_str_clipped;

use crate::{abbreviate_home, exit_if_terminated, restore_terminal};

/// Minimum column widths for the story list and info panel, respectively.
/// The panel refuses to open when the terminal is narrower than their sum.
const LIST_MIN_W: u16 = 24;
const PANEL_MIN_W: u16 = 28;

/// Story-list row layout: the selection-marker glyph column, the gap between
/// text columns, and each data column's target width. Year drops first as
/// the row narrows, then author, leaving title + badges at the narrowest —
/// see `compute_columns`.
const ROW_MARKER_W: u16 = 2;
const COL_GAP: u16 = 2;
const AUTHOR_COL_W: u16 = 20;
const YEAR_COL_W: u16 = 6;
const TITLE_MIN_W: u16 = 8;

/// Resolved column widths for one draw, given `text_w` — the row width left
/// for marker+title+author+year once the badge cluster's fixed columns (and
/// its lead-in gap) are excluded by the caller. Title always absorbs
/// whatever space the shown columns don't use, so there is never a gap
/// before the badges.
struct ListColumns {
    title_w: u16,
    author_w: u16,
    year_w: u16,
}

fn compute_columns(text_w: u16) -> ListColumns {
    let avail = text_w.saturating_sub(ROW_MARKER_W);
    let need_year = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W + COL_GAP + YEAR_COL_W;
    let need_author = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W;
    if avail >= need_year {
        ListColumns {
            title_w: avail - COL_GAP - AUTHOR_COL_W - COL_GAP - YEAR_COL_W,
            author_w: AUTHOR_COL_W,
            year_w: YEAR_COL_W,
        }
    } else if avail >= need_author {
        ListColumns { title_w: avail - COL_GAP - AUTHOR_COL_W, author_w: AUTHOR_COL_W, year_w: 0 }
    } else {
        ListColumns { title_w: avail, author_w: 0, year_w: 0 }
    }
}

/// Truncate `s` to at most `max_w` display columns (unicode display width,
/// not char count — a CJK title is 2 cells per char and `chars().count()`
/// would misalign every column to its right), appending `…` when it doesn't
/// fit.
fn truncate_to_width(s: &str, max_w: usize) -> String {
    if max_w == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max_w {
        return s.to_string();
    }
    if max_w == 1 {
        return "…".to_string();
    }
    let target = max_w - 1; // room for the 1-wide ellipsis
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > target {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// Column header text plus whether it's the active sort column — the
/// direction arrow is shown only on the active column.
fn header_label(name: &str, key: app::picker::SortKey, sort: app::picker::Sort) -> (String, bool) {
    if sort.key == key {
        let arrow = if sort.desc { "▼" } else { "▲" };
        (format!("{name} {arrow}"), true)
    } else {
        (name.to_string(), false)
    }
}

/// Optional footer hint segments, most-important (least-guessable) first.
/// Included left-to-right while they still fit next to the always-shown
/// core hints; the rest are dropped — `PgUp/PgDn` goes first since it's a
/// standard convention nobody needs told, `f`/`r` survive narrowest since
/// they name behavior no key convention predicts.
const FOOTER_OPTIONAL: [&str; 5] = ["f: fetch", "r: refresh", "s: sort", "d: reverse", "PgUp/PgDn"];

fn build_footer(width: u16) -> String {
    const CORE_LEFT: &str = " ↑/↓ or j/k: move";
    const CORE_RIGHT: &str = "Enter / click: open   i/Tab: info   q / Esc: quit";
    let mut footer = CORE_LEFT.to_string();
    for seg in FOOTER_OPTIONAL {
        let candidate = format!("{footer}   {seg}   {CORE_RIGHT}");
        if UnicodeWidthStr::width(candidate.as_str()) as u16 <= width {
            footer.push_str("   ");
            footer.push_str(seg);
        } else {
            break;
        }
    }
    footer.push_str("   ");
    footer.push_str(CORE_RIGHT);
    footer
}

/// True if the terminal is wide enough to show list + panel.
fn can_open_panel(width: u16) -> bool {
    width >= LIST_MIN_W + PANEL_MIN_W
}

/// Split `area` into (list, panel) given an eased open fraction in `[0,1]`.
/// Panel target width is a third of the area, clamped to
/// `[PANEL_MIN_W, area.width - LIST_MIN_W]`; the eased width is that × fraction.
fn split_picker_area(area: Rect, fraction: f64) -> (Rect, Rect) {
    if fraction <= 0.0 || !can_open_panel(area.width) {
        return (area, Rect::new(area.right(), area.y, 0, area.height));
    }
    let target = (area.width / 3).clamp(PANEL_MIN_W, area.width - LIST_MIN_W);
    let panel_w = ((target as f64) * fraction).round() as u16;
    let panel_w = panel_w.min(area.width - LIST_MIN_W);
    let list_w = area.width - panel_w;
    let list_area = Rect::new(area.x, area.y, list_w, area.height);
    let panel_area = Rect::new(area.x + list_w, area.y, panel_w, area.height);
    (list_area, panel_area)
}

/// Resolve and cache the aux data for `idx` if not already cached.
fn ensure_aux(
    cache: &mut [Option<app::picker::StoryAux>],
    stories: &[app::picker::StoryEntry],
    idx: usize,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) {
    if let Some(slot) = cache.get_mut(idx) {
        if slot.is_none() {
            if let Some(entry) = stories.get(idx) {
                *slot = Some(app::picker::resolve_aux(entry, data_base, hint_index));
            }
        }
    }
}

/// Build the ratatui-image picker for cover art per the CLI mode. `Auto`
/// queries the terminal (falling back to half-blocks); forced modes query for
/// font size then pin the protocol. Returns `None` only if construction fails.
pub(crate) fn build_cover_picker(mode: app::config::ImageProtocol) -> Option<ratatui_image::picker::Picker> {
    use app::config::ImageProtocol as M;
    use ratatui_image::picker::{Picker, ProtocolType};
    match mode {
        M::Halfblocks => Some(Picker::halfblocks()),
        M::Auto => Some(Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks())),
        M::Kitty | M::Sixel | M::Iterm2 => {
            let mut p = Picker::from_query_stdio().ok()?;
            p.set_protocol_type(match mode {
                M::Kitty => ProtocolType::Kitty,
                M::Sixel => ProtocolType::Sixel,
                M::Iterm2 => ProtocolType::Iterm2,
                _ => unreachable!(),
            });
            Some(p)
        }
    }
}

/// Run the pre-game story picker for a directory passed at launch. Returns the
/// chosen story path, or `None` if the user quit. Exits the process with a
/// message when the directory contains no launchable stories.
pub(crate) fn run_story_picker(
    dir: &std::path::Path,
    cfg: &app::config::Config,
    data_base: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let stories = app::picker::scan_stories(dir, data_base);
    if stories.is_empty() {
        eprintln!("babelmap: no Z-machine story files found in '{}'", dir.display());
        std::process::exit(1);
    }

    // Resolve themed colors the same way the game does, so the picker matches.
    let (base, _w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (cs, _set, _w2) = app::style::resolve(&base, &cfg.user_dir);

    // Row badges: each story's per-game dir under `data_base` + one shared hint
    // index, computed once (SQ-0284).
    let hint_index = app::hints::load_hint_index(&cfg.user_dir);
    let row_badges: Vec<app::picker::RowBadges> = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, &hint_index))
        .collect();
    let sym_cfg = app::style::finalize_symbols(&base.symbols);
    let badge_glyphs = app::picker::BadgeGlyphs::from_symbols(&sym_cfg);

    // Terminal setup mirrors the game loop. If any step fails we can't be
    // interactive — fall back to the first story rather than abort.
    if enable_raw_mode().is_err() {
        return Some(stories[0].path.clone());
    }
    if execute!(stdout(), EnterAlternateScreen).is_err() {
        let _ = disable_raw_mode();
        return Some(stories[0].path.clone());
    }
    // Mouse capture is opt-in (config `mouse = true`): its any-motion reporting
    // floods this loop with redraws on every mouse move. Off by default keeps the
    // browser snappy; click-to-select and wheel scroll require enabling it.
    if cfg.mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }
    let mut terminal = match Terminal::new(CrosstermBackend::new(stdout())) {
        Ok(t) => t,
        Err(_) => {
            restore_terminal();
            return Some(stories[0].path.clone());
        }
    };

    let cover_picker = if cfg.images { build_cover_picker(cfg.image_protocol) } else { None };
    let mut cover = app::cover::CoverState::default();

    let mut list = app::list_scroll::ListScroll::new();
    list.len(stories.len());
    let anim = &cfg.animation;
    let mut row_rects: Vec<(usize, Rect)> = Vec::new();
    let mut header_rects: Vec<(app::picker::SortKey, Rect)> = Vec::new();
    let mut viewport: usize = 0;

    // Info panel: always starts closed each launch (session-only state).
    let mut slide = PanelSlide::closed();
    let mut aux_cache: Vec<Option<app::picker::StoryAux>> =
        (0..stories.len()).map(|_| None).collect();
    let mut last_area = Rect::new(0, 0, 0, 0);
    let mut last_panel_area = Rect::new(0, 0, 0, 0);
    let mut panel_scroll: usize = 0;
    let mut panel_max: usize = 0;

    // Async cover decode: a background worker decodes off the main loop; results
    // are drained into `cover` each iteration. `requested` tracks in-flight paths
    // (so we don't re-queue), and the settle-debounce below waits until a
    // selection has been stable before requesting — a fling costs one decode.
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::time::Instant;
    let decoder = app::cover::CoverDecoder::new();
    let mut requested: HashSet<PathBuf> = HashSet::new();
    let mut last_sel = usize::MAX;
    let mut sel_changed_at = Instant::now();
    const COVER_DEBOUNCE: Duration = Duration::from_millis(90);
    // A physical wheel notch emits several events, all delivered to the input
    // buffer together. Record the direction here and apply exactly one selection
    // step once the buffer drains (at the loop top), so one notch = one story
    // regardless of how the terminal spaces the events within a notch.
    let mut pending_wheel: Option<isize> = None;

    let chosen: Option<std::path::PathBuf> = loop {
        // Restore the terminal + exit if an external termination signal arrived.
        exit_if_terminated();

        // Apply a coalesced wheel step once its notch's event burst has fully
        // drained from the input buffer (poll(0) empty). Separate notches are not
        // buffered together, so each still moves exactly one story.
        if let Some(d) = pending_wheel {
            if !crossterm::event::poll(Duration::ZERO).unwrap_or(false) {
                pending_wheel = None;
                panel_scroll = 0;
                list.move_by(d, viewport, anim);
            }
        }

        let _ = terminal.draw(|f| {
            let area = f.area();
            last_area = area;
            let buf = f.buffer_mut();
            let (list_area, panel_area) = split_picker_area(area, slide.fraction());
            // Task 9 wires an actual sort state through the keys/clicks; for
            // now this is always the Task 7 default (Title, ascending).
            let (rects, vp, hrects) = draw_story_picker(
                &stories, &list, &row_badges, &badge_glyphs, dir, &cs,
                app::picker::Sort::default(), list_area, buf,
            );
            row_rects = rects;
            viewport = vp;
            header_rects = hrects;
            if panel_area.width > 0 {
                if let Some(entry) = stories.get(list.selected) {
                    last_panel_area = panel_area;
                    panel_max = draw_info_panel(
                        &entry.title,
                        &entry.filename,
                        &entry.meta,
                        aux_cache[list.selected].as_ref(),
                        panel_scroll,
                        panel_area,
                        cover_picker.as_ref(),
                        &mut cover,
                        &entry.path,
                        slide.active(),
                        &cs,
                        buf,
                    );
                }
            }
        });

        // Housekeeping (runs every iteration, before the poll gate below, so a
        // timed-out tick still drains results and re-issues the debounced request).
        // Drain finished decodes into the multi-entry cache.
        let mut cover_arrived = false;
        for (path, img) in decoder.drain() {
            cover.insert(path.clone(), img);
            requested.remove(&path);
            cover_arrived = true;
        }
        if slide.open {
            ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
            let sel = stories[list.selected].path.clone();
            if list.selected != last_sel {
                last_sel = list.selected;
                sel_changed_at = Instant::now();
            }
            // Settle-debounce: only request once the selection has been stable, so a
            // fling through the list costs one decode instead of one per row.
            if app::cover::should_request_cover(
                cover.has(&sel),
                requested.contains(&sel),
                sel_changed_at.elapsed(),
                COVER_DEBOUNCE,
            ) {
                decoder.request(sel.clone());
                requested.insert(sel);
            }
        }

        // A decode just landed: loop back to redraw so the cover paints now. The
        // draw is at the top of the loop, and once the result is cached
        // `cover_busy` goes false — without this the loop would block on `read()`
        // and the new cover wouldn't appear until the next input event.
        if cover_arrived {
            list.finalize_if_done();
            continue;
        }

        // Tick while a scroll or panel-slide animation eases so the motion is
        // visible, or while a cover decode is in flight / still needed so results
        // drain and the debounced request fires without a keypress; otherwise
        // block until the next event.
        let sel_now = stories.get(list.selected).map(|e| &e.path);
        let cover_busy = slide.open
            && sel_now.is_some_and(|p| !requested.is_empty() || !cover.has(p));
        if (list.has_active_animation() || slide.active() || cover_busy)
            && !crossterm::event::poll(Duration::from_millis(16)).unwrap_or(false)
        {
            list.finalize_if_done();
            continue;
        }

        // Wait for the next event via a bounded poll instead of a plain blocking
        // read(): crossterm swallows the EINTR a signal delivers (and signal-hook
        // uses SA_RESTART), so an idle blocking read() would never observe the
        // termination flag. Re-check it each ~100ms tick (no redraw) so a
        // kill/SIGHUP restores the terminal promptly instead of hanging.
        loop {
            exit_if_terminated();
            match crossterm::event::poll(Duration::from_millis(100)) {
                Ok(true) => break,  // an event is ready → read it below
                Ok(false) => {}     // timeout → re-check the flag, keep waiting
                Err(_) => break,    // let read() below surface the error
            }
        }

        match read() {
            Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                use crossterm::event::KeyCode::*;
                let shift = k.modifiers.contains(crossterm::event::KeyModifiers::SHIFT);
                if slide.open && shift {
                    let page = (last_panel_area.height.saturating_sub(2)).max(1) as usize;
                    match k.code {
                        Up => panel_scroll = panel_scroll.saturating_sub(1),
                        Down => panel_scroll = (panel_scroll + 1).min(panel_max),
                        PageUp => panel_scroll = panel_scroll.saturating_sub(page),
                        PageDown => panel_scroll = (panel_scroll + page).min(panel_max),
                        _ => {}
                    }
                } else {
                    match k.code {
                        Up | Char('k') => { panel_scroll = 0; list.move_by(-1, viewport, anim) }
                        Down | Char('j') => { panel_scroll = 0; list.move_by(1, viewport, anim) }
                        PageUp => { panel_scroll = 0; list.page(-1, viewport, anim) }
                        PageDown => { panel_scroll = 0; list.page(1, viewport, anim) }
                        Home => { panel_scroll = 0; list.home(viewport, anim) }
                        End => { panel_scroll = 0; list.end(stories.len(), viewport, anim) }
                        Enter => break Some(stories[list.selected].path.clone()),
                        Esc | Char('q') => break None,
                        Char('i') | Tab => {
                            let target = !slide.open;
                            if !target || can_open_panel(last_area.width) {
                                let instant = !cfg.animation.enabled || cfg.animation.scroll_ms == 0;
                                slide.toggle_to(target, instant);
                                slide.arm(&cfg.animation);
                                if target {
                                    panel_scroll = 0;
                                    ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Mouse(m)) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    if let Some((idx, _)) = row_rects.iter().find(|(_, r)| r.contains(pt)) {
                        break Some(stories[*idx].path.clone());
                    }
                } else if let Some(d) = app::input::wheel_delta(m.kind, cfg.mouse_wheel_invert) {
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    if slide.open && last_panel_area.contains(pt) {
                        if d < 0 {
                            panel_scroll = panel_scroll.saturating_sub((-d) as usize);
                        } else {
                            panel_scroll = (panel_scroll + d as usize).min(panel_max);
                        }
                    } else {
                        // Record the notch's direction; the coalesced step is
                        // applied at the loop top once this notch's event burst
                        // drains, so one notch moves the selection one story.
                        pending_wheel = Some(d);
                    }
                }
            }
            Ok(Event::Resize(_, _)) => {
                let _ = terminal.clear();
            }
            Ok(_) => {}
            Err(_) => break None,
        }
        panel_scroll = panel_scroll.min(panel_max);
        list.finalize_if_done();
    };

    restore_terminal();
    chosen
}

/// Draw the story-picker screen. Returns the per-row hit-rects (index, rect)
/// for mouse selection, the row count, and the column-header hit-rects
/// (Task 9 hit-tests these for click-to-sort).
fn draw_story_picker(
    stories: &[app::picker::StoryEntry],
    list: &app::list_scroll::ListScroll,
    badges: &[app::picker::RowBadges],
    glyphs: &app::picker::BadgeGlyphs,
    dir: &std::path::Path,
    cs: &app::colors::ColorScheme,
    sort: app::picker::Sort,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) -> (Vec<(usize, Rect)>, usize, Vec<(app::picker::SortKey, Rect)>) {
    use app::picker::SortKey;
    use ratatui::style::{Color, Style};
    let selected = list.selected;
    let mut row_rects: Vec<(usize, Rect)> = Vec::new();
    let mut header_rects: Vec<(SortKey, Rect)> = Vec::new();

    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(cs.dialog);
            }
        }
    }

    // Header.
    let header = format!(
        " babelmap — choose a story  ({} found in {})   [i: info]",
        stories.len(),
        dir.display()
    );
    draw_str_clipped(buf, area.x, area.y, &header, cs.dialog_title, area);

    // List region (title bar + column-header row at top, footer at bottom).
    let list_top = area.y + 2;
    let list_bottom = area.bottom().saturating_sub(1);
    if list_bottom <= list_top {
        return (row_rects, 0, header_rects);
    }
    let rows = (list_bottom - list_top) as usize;
    let total = stories.len();

    // Reserve a 1-col gutter for the scrollbar when the list overflows.
    let scrollbar_visible =
        app::render::scroll::needs_scrollbar(total, rows) && area.width >= 2;
    let row_w = if scrollbar_visible { area.width.saturating_sub(1) } else { area.width };
    let first = list.display_offset();

    // Badge cluster width depends only on the configured glyphs, not the
    // entry, so it's computed once and reused both to size the text columns
    // and to place each row's cluster.
    let type_w = glyphs.zcode.chars().count().max(glyphs.glulx.chars().count()) as u16;
    let blorb_w = glyphs.blorb.chars().count() as u16;
    let save_w = glyphs.save.chars().count() as u16;
    let hint_w = glyphs.hint.chars().count() as u16;
    let cluster_w = type_w + blorb_w + save_w + hint_w;
    let badges_shown = cluster_w + 2 < row_w;
    let badge_reserved = if badges_shown { cluster_w + 1 } else { 0 };
    let text_w = row_w.saturating_sub(badge_reserved);
    let cols = compute_columns(text_w);

    let title_x = area.left() + ROW_MARKER_W;
    let author_x = title_x + cols.title_w + COL_GAP;
    let year_x = author_x + cols.author_w + COL_GAP;

    // Column-header row: dimmed, except the active sort column, which shows
    // its direction arrow.
    let header_y = area.y + 1;
    let (title_label, title_active) = header_label("TITLE", SortKey::Title, sort);
    let title_hstyle = if title_active { cs.story_header_active } else { cs.story_header };
    draw_str_clipped(buf, title_x, header_y, &title_label, title_hstyle, area);
    header_rects.push((SortKey::Title, Rect::new(title_x, header_y, cols.title_w, 1)));
    if cols.author_w > 0 {
        let (author_label, author_active) = header_label("AUTHOR", SortKey::Author, sort);
        let author_hstyle = if author_active { cs.story_header_active } else { cs.story_header };
        draw_str_clipped(buf, author_x, header_y, &author_label, author_hstyle, area);
        header_rects.push((SortKey::Author, Rect::new(author_x, header_y, cols.author_w, 1)));
    }
    if cols.year_w > 0 {
        let (year_label, year_active) = header_label("YEAR", SortKey::Year, sort);
        let year_hstyle = if year_active { cs.story_header_active } else { cs.story_header };
        draw_str_clipped(buf, year_x, header_y, &year_label, year_hstyle, area);
        header_rects.push((SortKey::Year, Rect::new(year_x, header_y, cols.year_w, 1)));
    }

    for (i, entry) in stories.iter().enumerate().skip(first).take(rows) {
        let y = list_top + (i - first) as u16;
        let row_rect = Rect::new(area.x, y, row_w, 1);
        row_rects.push((i, row_rect));
        let sel = i == selected;
        let style = if sel { cs.dialog_button_active } else { cs.dialog };
        for x in area.left()..area.left() + row_w {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(style);
            }
        }
        let marker = if sel { "▸ " } else { "  " };
        draw_str_clipped(buf, area.x, y, marker, style, row_rect);

        let title_txt = truncate_to_width(&entry.title, cols.title_w as usize);
        draw_str_clipped(buf, title_x, y, &title_txt, style, row_rect);

        if cols.author_w > 0 {
            let (author_txt, author_style) = match entry.meta.author.as_deref() {
                Some(a) if !a.is_empty() => {
                    (truncate_to_width(a, cols.author_w as usize), cs.story_author)
                }
                _ => (
                    truncate_to_width("(no metadata yet)", cols.author_w as usize),
                    cs.story_no_metadata,
                ),
            };
            // Selection highlight wins over the column's own color, same as
            // the title text above — the whole row reads as one bar.
            let author_style = if sel { style } else { author_style };
            draw_str_clipped(buf, author_x, y, &author_txt, author_style, row_rect);
        }

        if cols.year_w > 0 {
            if let Some(yr) = entry.meta.year.as_deref().filter(|s| !s.is_empty()) {
                let year_txt = truncate_to_width(yr, cols.year_w as usize);
                let year_style = if sel { style } else { cs.story_year };
                draw_str_clipped(buf, year_x, y, &year_txt, year_style, row_rect);
            }
        }

        // Right-aligned badge cluster: fixed columns for [type][blorb][save][hint],
        // no separators, so present badges stay vertically aligned across rows.
        let b = badges.get(i).copied().unwrap_or_default();
        let type_glyph = match entry.meta.engine {
            app::picker::Engine::ZCode => glyphs.zcode,
            app::picker::Engine::Glulx => glyphs.glulx,
        };
        if badges_shown {
            let bx = area.left() + row_w - 1 - cluster_w;
            // On the selection bar the plain badge fg (e.g. green) is low-contrast
            // against the highlight, so reverse it into a block: the badge colour
            // becomes the background and the selection bar's text colour the glyph
            // — readable and still distinct. Unselected rows keep plain letters.
            // Blank slots are left untouched so they show the plain selection bar,
            // not a green block.
            let badge_style = if sel {
                Style::new()
                    .fg(cs.dialog_button_active.fg.unwrap_or(Color::Reset))
                    .bg(cs.story_badge.fg.unwrap_or(Color::Reset))
            } else {
                cs.story_badge
            };
            draw_str_clipped(buf, bx, y, type_glyph, badge_style, row_rect);
            if b.blorb {
                draw_str_clipped(buf, bx + type_w, y, glyphs.blorb, badge_style, row_rect);
            }
            if b.save {
                draw_str_clipped(buf, bx + type_w + blorb_w, y, glyphs.save, badge_style, row_rect);
            }
            if b.hint {
                draw_str_clipped(buf, bx + type_w + blorb_w + save_w, y, glyphs.hint, badge_style, row_rect);
            }
        }
    }

    if scrollbar_visible {
        let sb_area = Rect::new(area.right().saturating_sub(1), list_top, 1, rows as u16);
        app::render::scroll::draw_scrollbar(buf, sb_area, total, rows, list.target_offset(), cs.scrollbar);
    }

    // Footer hint.
    let footer = build_footer(area.width);
    let fstyle = Style::new().fg(Color::DarkGray).patch(cs.dialog);
    draw_str_clipped(buf, area.x, list_bottom, &footer, fstyle, area);

    (row_rects, rows, header_rects)
}

/// Draw the highlighted story's metadata panel: title, filesystem info,
/// format/version/release, serial (Z only), IFID, present features, bundled
/// resources (self-blorb or an associated sibling blorb), and saves. Pure
/// renderer — no state, no interaction (the picker loop wires toggling/
/// slide/lazy-resolve).
fn draw_info_panel(
    title: &str,
    filename: &str,
    meta: &app::picker::StoryMeta,
    aux: Option<&app::picker::StoryAux>,
    scroll: usize,
    area: Rect,
    picker: Option<&ratatui_image::picker::Picker>,
    cover: &mut app::cover::CoverState,
    entry_path: &std::path::Path,
    animating: bool,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
) -> usize {
    if area.width < 2 || area.height < 2 {
        return 0;
    }
    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(cs.story_info);
            }
        }
    }

    // Single-line border box titled " Info ".
    let frame = app::render::paneframe::draw_pane_frame(
        buf,
        area,
        app::render::paneframe::BorderStyle::Single,
        &app::render::paneframe::PaneGlyphs::default(),
        cs.story_info,
    );
    draw_str_clipped(buf, area.x + 2, area.y, " Info ", cs.story_info_title, area);

    let mut inner = frame.content;

    // Cover band: top of the panel, ≤50% of the panel's inner height is the
    // *maximum* fit box; the actual band is sized down to the image's
    // aspect-fitted height so no dead letterbox rows push the info text down.
    // Only drawn when the selected story has a decoded frontispiece and a
    // picker exists.
    if let Some(picker) = picker {
        if cover.has(entry_path) {
            let cover_h = (inner.height / 2).min(inner.height.saturating_sub(1));
            if cover_h >= 1 {
                let cover_area = Rect::new(inner.x, inner.y, inner.width, cover_h);
                let mut used_h = 0u16;
                if let Some(proto) = cover.protocol(picker, entry_path, cover_area, animating) {
                    // Fitted (aspect-preserved) size, clamped to the max box.
                    let sz = proto.size();
                    let used_w = sz.width.min(inner.width);
                    used_h = sz.height.min(cover_h);
                    // Themed letterbox fill, sized to the actual fitted band
                    // (not the full max box) so there's no dead space below.
                    let fill_area = Rect::new(cover_area.x, cover_area.y, cover_area.width, used_h);
                    for y in fill_area.top()..fill_area.bottom() {
                        for x in fill_area.left()..fill_area.right() {
                            if let Some(c) = buf.cell_mut((x, y)) {
                                c.set_symbol(" ").set_style(cs.story_info_cover);
                            }
                        }
                    }
                    // Top-aligned, horizontally centered within the band.
                    let dest = Rect::new(
                        cover_area.x + (inner.width - used_w) / 2,
                        cover_area.y,
                        used_w,
                        used_h,
                    );
                    ratatui::widgets::Widget::render(
                        ratatui_image::Image::new(proto),
                        dest,
                        buf,
                    );
                }
                if used_h > 0 {
                    inner = Rect::new(inner.x, inner.y + used_h, inner.width, inner.height - used_h);
                }
            }
        }
    }

    let mut lines: Vec<(String, ratatui::style::Style)> = Vec::new();

    // Title.
    lines.push((title.to_string(), cs.story_info_title));
    // filename · size · modified.
    let mut fs_line = format!("{} · {}", filename, human_size(meta.size_bytes));
    if let Some(m) = &meta.modified {
        fs_line.push_str(&format!(" · {m}"));
    }
    lines.push((fs_line, cs.story_info_value));
    // format + version · release.
    let mut fmt_line = meta.format.clone();
    if let Some(v) = &meta.version {
        fmt_line = match meta.engine {
            app::picker::Engine::ZCode => format!("{} v{}", meta.format, v),
            app::picker::Engine::Glulx => format!("{} {}", meta.format, v),
        };
    }
    if let Some(r) = meta.release {
        fmt_line.push_str(&format!(" · Release {r}"));
    }
    lines.push((fmt_line, cs.story_info_value));
    // serial (Z only).
    if let Some(s) = &meta.serial {
        lines.push((format!("Serial {s}"), cs.story_info_value));
    }
    // ifid.
    lines.push((format!("IFID {}", meta.ifid), cs.story_info_value));
    // features line (present badges only).
    let feats = feature_words(&meta.features, aux);
    if !feats.is_empty() {
        lines.push((format!("Features: {}", feats.join(" ")), cs.story_info_value));
    }

    // Saves + sidecars (SQ-0285). Rendered above Resources so the user's own
    // saves are the first thing they see below the metadata.
    if let Some(a) = aux {
        let has_any = !a.saves.is_empty() || !a.qzl_saves.is_empty()
            || !a.auto_saves.is_empty() || !a.sidecars.is_empty();
        if has_any {
            lines.push((String::new(), cs.story_info_value));
            // Header: "Saves · <dir>" with $HOME abbreviated to ~.
            let dir = abbreviate_home(&a.game_dir);
            lines.push((format!("Saves · {dir}"), cs.story_info_label));
            for s in &a.saves {
                let when = s.saved_at.get(0..10).unwrap_or(&s.saved_at);
                let fname = s.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                lines.push((format!(" {}  turn {} · {}  {}", s.name, s.turns, when, fname), cs.story_info_value));
            }
            for q in &a.qzl_saves {
                let when = q.saved_at.get(0..10).unwrap_or(&q.saved_at);
                let fname = q.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                lines.push((format!(" {}  {}  {}", q.name, when, fname), cs.story_info_value));
            }
            if !a.auto_saves.is_empty() {
                lines.push(("Automatic:".to_string(), cs.story_info_label));
                for q in &a.auto_saves {
                    let when = q.saved_at.get(0..10).unwrap_or(&q.saved_at);
                    let fname = q.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    lines.push((format!(" (auto) {}  {}  {}", q.name, when, fname), cs.story_info_value));
                }
            }
            if !a.sidecars.is_empty() {
                lines.push((format!("Sidecars: {}", a.sidecars.join(" · ")), cs.story_info_value));
            }
        }
    }

    // Resources: self_blorb, else aux.assoc_blorb.
    let (res_header, chunks): (Option<String>, &[app::picker::ChunkInfo]) =
        if let Some(c) = &meta.self_blorb {
            (Some(format!("Resources ({filename})")), c.as_slice())
        } else if let Some((src, c)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
            let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("blorb");
            (Some(format!("Resources ({name})")), c.as_slice())
        } else {
            (None, &[])
        };
    if let Some(h) = res_header {
        lines.push((String::new(), cs.story_info_value));
        lines.push((h, cs.story_info_label));
        for c in chunks {
            let base = format!(
                " #{}  {} — {}",
                c.number,
                resource_usage_label(&c.usage),
                resource_type_label(&c.chunk_type),
            );
            let line = match &c.detail {
                Some(d) => format!("{base} · {d} ({})", human_size(c.len as u64)),
                None => format!("{base} ({})", human_size(c.len as u64)),
            };
            lines.push((line, cs.story_info_value));
        }
    }

    // Reserve a 1-col gutter for the scrollbar when content overflows.
    let overflow = lines.len() as u16 > inner.height;
    let text_area = if overflow {
        Rect::new(inner.x, inner.y, inner.width.saturating_sub(1), inner.height)
    } else {
        inner
    };
    let content_height = inner.height as usize;
    let max_scroll = lines.len().saturating_sub(content_height);
    let eff = scroll.min(max_scroll);
    let end = (eff + content_height).min(lines.len());
    for (i, (text, style)) in lines[eff..end].iter().enumerate() {
        let y = inner.y + i as u16;
        draw_str_clipped(buf, text_area.x, y, text, *style, text_area);
    }
    if overflow {
        let sb_area = Rect::new(inner.right().saturating_sub(1), inner.y, 1, inner.height);
        app::render::scroll::draw_scrollbar(buf, sb_area, lines.len(), inner.height as usize, eff, cs.scrollbar);
    }
    max_scroll
}

/// Translate a raw Blorb resource usage FourCC into a human-readable label.
fn resource_usage_label(usage: &str) -> String {
    match usage.trim() {
        "Exec" => "Code".into(),
        "Pict" => "Image".into(),
        "Snd" => "Sound".into(),
        "Data" => "Data".into(),
        other => other.to_string(), // unknown: show raw (trimmed), nothing hidden
    }
}

/// Translate a raw Blorb chunk-type FourCC into a human-readable label.
fn resource_type_label(chunk_type: &str) -> String {
    match chunk_type.trim() {
        "ZCOD" => "Z-code".into(),
        "GLUL" => "Glulx".into(),
        "FORM" => "AIFF".into(),
        "OGGV" => "Ogg Vorbis".into(),
        "MOD" => "MOD".into(),
        "PNG" => "PNG".into(),
        "JPEG" => "JPEG".into(),
        "GIF" => "GIF".into(),
        other => other.to_string(), // unknown: raw FourCC
    }
}

/// Format a byte count as `"N B"` / `"N KB"` / `"N.N MB"`.
fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

/// Present-only feature badge words, folding in aux-derived signals (an
/// associated blorb's sound/picture chunks, or a resolved hint index).
fn feature_words(f: &app::picker::Features, aux: Option<&app::picker::StoryAux>) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut sound = f.sound;
    let mut graphics = f.graphics;
    if let Some((_, chunks)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
        if chunks.iter().any(|c| c.usage == "Snd ") {
            sound = true;
        }
        if chunks.iter().any(|c| c.usage == "Pict") {
            graphics = true;
        }
    }
    if sound {
        v.push("sound");
    }
    if graphics {
        v.push("graphics");
    }
    if f.colour == Some(true) {
        v.push("colour");
    }
    if f.hints || aux.map(|a| a.hints_available).unwrap_or(false) {
        v.push("hints");
    }
    v
}

#[cfg(test)]
mod tests {
    // ── Story-picker row badges (type + present artifacts) ─────────────────────

    fn row_text(buf: &ratatui::buffer::Buffer, y: u16, area: ratatui::layout::Rect) -> String {
        (area.left()..area.right())
            .filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string()))
            .collect()
    }

    /// `needle`'s CHAR (column) index within `row_text`'s output, not its byte
    /// index — a preceding multi-byte cell (e.g. the "▸" selection marker)
    /// would otherwise overcount a plain `.find()`.
    fn char_pos(row: &str, needle: &str) -> usize {
        let byte_idx = row.find(needle).unwrap_or_else(|| panic!("{needle:?} not found in {row:?}"));
        row[..byte_idx].chars().count()
    }

    fn make_two_test_stories() -> Vec<app::picker::StoryEntry> {
        use app::picker::{Engine, Features, StoryEntry, StoryMeta};
        let mk = |title: &str, engine: Engine| StoryEntry {
            path: std::path::PathBuf::from(format!("/tmp/{title}.z5")),
            title: title.into(),
            filename: format!("{title}.z5"),
            meta: StoryMeta {
                size_bytes: 1, modified: None, engine, format: "Z-code".into(),
                version: None, serial: None, release: None, ifid: title.into(),
                features: Features::default(), self_blorb: None,
                author: None, year: None, genre: None, language: None, description: None,
            },
        };
        vec![mk("Zork", Engine::ZCode), mk("Anchorhead", Engine::Glulx)]
    }

    /// Build a story entry with an explicit author/year (or none), for the
    /// column-layout tests below.
    fn story_with_meta(title: &str, author: Option<&str>, year: Option<&str>) -> app::picker::StoryEntry {
        use app::picker::{Engine, Features, StoryEntry, StoryMeta};
        StoryEntry {
            path: std::path::PathBuf::from(format!("/tmp/{title}.z5")),
            title: title.into(),
            filename: format!("{title}.z5"),
            meta: StoryMeta {
                size_bytes: 1, modified: None, engine: Engine::ZCode, format: "Z-code".into(),
                version: None, serial: None, release: None, ifid: title.into(),
                features: Features::default(), self_blorb: None,
                author: author.map(String::from), year: year.map(String::from),
                genre: None, language: None, description: None,
            },
        }
    }

    #[test]
    fn row_renders_type_badge_and_present_artifacts() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        // One Z-code story with all three artifacts, one Glulx story with only a save.
        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: true, hint: true },
            app::picker::RowBadges { blorb: false, save: true, hint: false },
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());

        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        let dir = std::path::Path::new("/tmp");
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, dir, &cs, app::picker::Sort::default(), area, &mut buf,
        );

        let row0 = row_text(&buf, 2, area); // list starts at area.y + 2
        let row1 = row_text(&buf, 3, area);
        assert!(row0.contains("ZBSH"), "adjacent, no separators: {row0:?}");
        assert!(row1.contains("S"), "got: {row1:?}");
        assert!(!row1.contains("B"), "absent blorb omitted: {row1:?}");
        assert!(!row1.contains("H"), "absent hint omitted: {row1:?}");

        // Fixed-slot alignment: the save glyph must land at the same column
        // in both rows regardless of which other artifacts are present.
        // (char index, not byte index — row0's "▸ " marker is multi-byte.)
        let save_x0 = row0.chars().position(|c| c == 'S').expect("row0 has save glyph");
        let save_x1 = row1.chars().position(|c| c == 'S').expect("row1 has save glyph");
        assert_eq!(save_x0, save_x1, "save glyph column must be fixed across rows");
    }

    #[test]
    fn row_uses_configured_badge_glyphs() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut sym = app::config::SymbolConfig::default();
        sym.badge_zcode = "z!".into();
        sym.badge_blorb = "◆".into();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: false, hint: false },
            app::picker::RowBadges::default(),
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(&stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
                          &cs, app::picker::Sort::default(), area, &mut buf);
        let row0 = row_text(&buf, 2, area);
        assert!(row0.contains("z!◆"), "configured glyphs used, no separators: {row0:?}");
    }

    // ── Story-picker list: columns, header, sort ────────────────────────────────

    #[test]
    fn header_row_shows_columns_and_active_direction_arrow() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("Curses", Some("Graham Nelson"), Some("1993")),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);

        // Default sort (Title, ascending): only TITLE carries an arrow.
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        let header = row_text(&buf, 1, area); // header row is area.y + 1
        assert!(header.contains("TITLE ▲"), "active column shows the ascending arrow: {header:?}");
        assert!(header.contains("AUTHOR"), "author header present: {header:?}");
        assert!(!header.contains("AUTHOR ▲") && !header.contains("AUTHOR ▼"), "inactive column has no arrow: {header:?}");
        assert!(header.contains("YEAR"), "year header present: {header:?}");
        assert!(!header.contains("YEAR ▲") && !header.contains("YEAR ▼"), "inactive column has no arrow: {header:?}");

        // Sort by Year, descending: only YEAR carries the down arrow.
        let mut buf2 = Buffer::empty(area);
        let sort2 = app::picker::Sort { key: app::picker::SortKey::Year, desc: true };
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, sort2, area, &mut buf2,
        );
        let header2 = row_text(&buf2, 1, area);
        assert!(header2.contains("YEAR ▼"), "active column shows the descending arrow: {header2:?}");
        assert!(!header2.contains("TITLE ▲") && !header2.contains("TITLE ▼"), "{header2:?}");
    }

    #[test]
    fn row_renders_author_and_year_aligned_across_rows() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("Curses", Some("Graham Nelson"), Some("1993")),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        let row0 = row_text(&buf, 2, area);
        let row1 = row_text(&buf, 3, area);
        assert!(row0.contains("Michael S. Gentry"), "{row0:?}");
        assert!(row0.contains("1998"), "{row0:?}");
        assert!(row1.contains("Graham Nelson"), "{row1:?}");
        assert!(row1.contains("1993"), "{row1:?}");

        let author_x0 = char_pos(&row0, "Michael");
        let author_x1 = char_pos(&row1, "Graham");
        assert_eq!(author_x0, author_x1, "author column must align across rows");
        let year_x0 = char_pos(&row0, "1998");
        let year_x1 = char_pos(&row1, "1993");
        assert_eq!(year_x0, year_x1, "year column must align across rows");
    }

    #[test]
    fn row_with_no_author_shows_no_metadata_placeholder_styled_correctly() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        // A fresh bare-z file with no fetched/embedded metadata — the common
        // case for a library nobody has run a fetch on yet, not an edge case.
        // A second, unrelated story keeps the no-metadata row UNSELECTED
        // (selection highlight intentionally overrides column colors, same
        // as the badge cluster does — so this checks the plain-row style).
        let stories = vec![
            story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998")),
            story_with_meta("zork2-r63-s860811", None, None),
        ];
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(2);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        let row1 = row_text(&buf, 3, area);
        assert!(row1.contains("(no metadata yet)"), "reads as 'nothing fetched yet': {row1:?}");

        // Styled via cs.story_no_metadata, not cs.story_author — terminal_default
        // gives them distinct fg colors (DarkGray vs White), so this checks the
        // right field was actually applied, not just that text is present.
        let x = char_pos(&row1, "(no metadata yet)") as u16;
        let cell = buf.cell((area.left() + x, 3)).unwrap();
        assert_eq!(cell.fg, cs.story_no_metadata.fg.unwrap(), "placeholder must use story_no_metadata's color");
        assert_ne!(cs.story_no_metadata.fg, cs.story_author.fg, "sanity: the two styles must actually differ");
    }

    #[test]
    fn columns_drop_year_then_author_as_width_narrows() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let cluster_w: u16 = 4; // default glyphs: 1-col type + blorb + save + hint

        // (width, author shown, year shown) — thresholds per compute_columns:
        // year needs width >= 45, author needs width >= 37, below that:
        // title + badges only. No width in between should show a gap: the
        // badge cluster's column (checked below) never moves off its
        // width-derived formula regardless of which text columns show.
        for &(width, want_author, want_year) in &[
            (60u16, true, true),
            (45, true, true),
            (44, true, false),
            (37, true, false),
            (36, false, false),
            (30, false, false),
        ] {
            let area = Rect::new(0, 0, width, 10);
            let mut buf = Buffer::empty(area);
            super::draw_story_picker(
                &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
                &cs, app::picker::Sort::default(), area, &mut buf,
            );
            let row = row_text(&buf, 2, area);
            assert_eq!(row.contains("Michael S. Gentry"), want_author, "width {width}: {row:?}");
            assert_eq!(row.contains("1998"), want_year, "width {width}: {row:?}");

            // Badges stay right-aligned at the same formula regardless of
            // which text columns are shown — proves no gap opened in front
            // of them as columns drop.
            let bx = width - 1 - cluster_w;
            let cell = buf.cell((bx, 2)).unwrap();
            assert_eq!(cell.symbol(), "Z", "badge cluster must start at col {bx} for width {width}: {row:?}");
        }
    }

    #[test]
    fn long_author_truncates_with_ellipsis_within_column() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let long_author = "Marc Blank and Dave Lebling and a Whole Lot More People";
        let stories = vec![story_with_meta("Zork I", Some(long_author), Some("1980"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        let row0 = row_text(&buf, 2, area);
        assert!(!row0.contains(long_author), "long author must be truncated: {row0:?}");
        assert!(row0.contains('…'), "truncated author ends with an ellipsis: {row0:?}");
        assert!(row0.contains("1980"), "year column unaffected by the author overrun: {row0:?}");
        let bx = 60u16 - 1 - 4;
        assert_eq!(
            buf.cell((bx, 2)).unwrap().symbol(), "Z",
            "badge cluster unaffected by the author overrun"
        );
    }

    #[test]
    fn header_rects_line_up_with_drawn_header_text() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = vec![story_with_meta("Anchorhead", Some("Michael S. Gentry"), Some("1998"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        let (_, _, header_rects) = super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        assert_eq!(header_rects.len(), 3, "all three columns are shown at this width: {header_rects:?}");
        for (key, rect) in &header_rects {
            let expected_char = match key {
                app::picker::SortKey::Title => "T",
                app::picker::SortKey::Author => "A",
                app::picker::SortKey::Year => "Y",
            };
            let cell = buf.cell((rect.x, rect.y)).unwrap();
            assert_eq!(
                cell.symbol(), expected_char,
                "{key:?} rect at ({}, {}) must start where its header text is actually drawn",
                rect.x, rect.y
            );
        }
    }

    #[test]
    fn footer_hints_drop_right_to_left_keeping_f_and_r_longest() {
        // Narrow: none of the new hints fit, but the existing core (move/open/
        // info/quit) is always present.
        let narrow = super::build_footer(60);
        assert!(narrow.contains("move") && narrow.contains("open") && narrow.contains("info") && narrow.contains("quit"));
        assert!(!narrow.contains("f: fetch") && !narrow.contains("PgUp/PgDn"), "{narrow:?}");

        // f/r (least guessable) survive down to the narrowest widths that fit
        // them at all; s/d/PgUp/PgDn need progressively more room.
        let at_80 = super::build_footer(80);
        assert!(at_80.contains("f: fetch"), "{at_80:?}");
        assert!(!at_80.contains("r: refresh"), "{at_80:?}");

        let at_93 = super::build_footer(93);
        assert!(at_93.contains("r: refresh") && !at_93.contains("s: sort"), "{at_93:?}");

        let at_103 = super::build_footer(103);
        assert!(at_103.contains("s: sort") && !at_103.contains("d: reverse"), "{at_103:?}");

        let at_116 = super::build_footer(116);
        assert!(at_116.contains("d: reverse") && !at_116.contains("PgUp/PgDn"), "{at_116:?}");

        let at_128 = super::build_footer(128);
        assert!(at_128.contains("PgUp/PgDn"), "{at_128:?}");
    }

    // ── Story-picker info panel ─────────────────────────────────────────────────

    fn buffer_to_string(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> String {
        let mut out = String::new();
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(c) = buf.cell((x, y)) {
                    out.push_str(c.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn info_panel_renders_metadata_features_resources_and_saves() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = app::picker::StoryMeta {
            size_bytes: 92 * 1024,
            modified: Some("2026-06-30".into()),
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: Some("840726".into()),
            release: Some(88),
            ifid: "ZCODE-88-840726".into(),
            features: app::picker::Features { sound: true, graphics: true, colour: Some(false), hints: true },
            self_blorb: Some(vec![
                app::picker::ChunkInfo { usage: "Exec".into(), number: 0, chunk_type: "ZCOD".into(), len: 92 * 1024, detail: None },
                app::picker::ChunkInfo {
                    usage: "Snd ".into(), number: 32, chunk_type: "FORM".into(), len: 12 * 1024,
                    detail: Some("15.4 kHz · 8-bit · mono · 2.2s".into()),
                },
            ]),
            author: None, year: None, genre: None, language: None, description: None,
        };
        let game_dir = std::path::PathBuf::from("/tmp/babelmap-info-panel-saves/zork1.z3");
        let aux = app::picker::StoryAux {
            assoc_blorb: None,
            saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("before-troll.babelmap"),
                name: "before-troll".into(),
                turns: 42,
                saved_at: "2026-06-30T00:00:00Z".into(),
                is_default: false,
            }],
            hints_available: false,
            game_dir: game_dir.clone(),
            qzl_saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("quick.qzl"),
                name: "quick".into(),
                turns: 0,
                saved_at: "2026-06-29T00:00:00Z".into(),
                is_default: false,
            }],
            auto_saves: vec![app::persist_files::SaveInfo {
                path: game_dir.join("_startup.qzl"),
                name: "_startup".into(),
                turns: 0,
                saved_at: "2026-06-28T00:00:00Z".into(),
                is_default: false,
            }],
            sidecars: vec!["default.aux"],
        };
        // Wide enough that the resource detail suffix isn't clipped.
        let area = Rect::new(0, 0, 70, 25);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, Some(&aux), 0, area, None, &mut cover, entry_path, false, &cs, &mut buf,
        );

        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Zork I"), "title line: {text:?}");
        assert!(text.contains("zork1.z3"), "filename: {text:?}");
        assert!(text.contains("Z-code"), "format line: {text:?}");
        assert!(text.contains("Release 88"));
        assert!(text.contains("840726"));
        assert!(text.contains("ZCODE-88-840726"));
        assert!(text.contains("sound"));
        assert!(text.contains("graphics"));
        assert!(text.contains("hints"));
        assert!(text.contains("Code"));
        assert!(text.contains("Sound"));
        assert!(text.contains("AIFF"));
        assert!(text.contains("15.4 kHz · 8-bit · mono · 2.2s"), "parsed detail: {text:?}");
        assert!(text.contains("Saves ·"), "saves dir header: {text:?}");
        assert!(text.contains("before-troll.babelmap"), "babelmap filename: {text:?}");
        assert!(text.contains("quick.qzl"), "qzl filename: {text:?}");
        assert!(text.contains("Sidecars:"), "sidecars line: {text:?}");
        assert!(text.contains("default.aux"), "sidecar filename: {text:?}");
        // SQ-0285-b: auto (game-managed) saves render, clearly labeled.
        assert!(text.contains("(auto)"), "auto-save label: {text:?}");
        assert!(text.contains("_startup.qzl"), "auto-save filename: {text:?}");
        // SQ-0285-b: Saves section now renders ABOVE Resources.
        let saves_pos = text.find("Saves ·").expect("saves header present");
        let resources_pos = text.find("Resources").expect("resources header present");
        assert!(saves_pos < resources_pos, "Saves must render before Resources: saves@{saves_pos} resources@{resources_pos}");
    }

    #[test]
    fn info_panel_scrolls_to_reveal_overflow() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let chunks: Vec<app::picker::ChunkInfo> = (0..30)
            .map(|i| app::picker::ChunkInfo {
                usage: "Data".into(),
                number: i,
                chunk_type: "IFhd".into(),
                len: 128,
                detail: None,
            })
            .collect();
        let meta = app::picker::StoryMeta {
            size_bytes: 92 * 1024,
            modified: None,
            engine: app::picker::Engine::ZCode,
            format: "Z-code".into(),
            version: Some("3".into()),
            serial: None,
            release: None,
            ifid: "ZCODE-88-840726".into(),
            features: app::picker::Features::default(),
            self_blorb: Some(chunks),
            author: None, year: None, genre: None, language: None, description: None,
        };
        let area = Rect::new(0, 0, 34, 10);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        let max_scroll = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf,
        );
        let text_top = buffer_to_string(&buf, area);
        assert!(max_scroll > 0, "content should overflow a 10-row panel");
        let late_marker = " #29  ";
        assert!(!text_top.contains(late_marker), "late resource should be offscreen at scroll 0: {text_top:?}");

        let mut buf2 = Buffer::empty(area);
        let max_scroll2 = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, max_scroll, area, None, &mut cover, entry_path, false, &cs, &mut buf2,
        );
        let text_scrolled = buffer_to_string(&buf2, area);
        assert_eq!(max_scroll2, max_scroll);
        assert!(text_scrolled.contains(late_marker), "late resource should be visible when scrolled: {text_scrolled:?}");

        // Scrolling past max clamps to the same view as scroll == max_scroll.
        let mut buf3 = Buffer::empty(area);
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 999, area, None, &mut cover, entry_path, false, &cs, &mut buf3,
        );
        let text_over = buffer_to_string(&buf3, area);
        assert_eq!(text_over, text_scrolled, "scroll past max should clamp to max_scroll view");
    }

    fn minimal_story_meta() -> app::picker::StoryMeta {
        app::picker::StoryMeta {
            size_bytes: 1, modified: None, engine: app::picker::Engine::Glulx,
            format: "Blorb (Glulx)".into(), version: Some("3.1.2".into()),
            serial: None, release: None, ifid: "IFID-X".into(),
            features: app::picker::Features::default(), self_blorb: None,
            author: None, year: None, genre: None, language: None, description: None,
        }
    }

    #[test]
    fn info_panel_renders_cover_band_when_present() {
        use ratatui::layout::Rect;
        use ratatui::buffer::Buffer;

        // A tiny valid PNG (via the image crate) as the decoded cover.
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([200, 50, 50]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let path = std::path::PathBuf::from("cover-test.gblorb");
        let mut cover = app::cover::CoverState::default();
        cover.insert(path.to_path_buf(), app::cover::decode(&png));

        // Deterministic, terminal-free protocol.
        let picker = ratatui_image::picker::Picker::halfblocks();

        // Mirror draw_story_picker_full_width_then_split for cs + buffer setup.
        let cs = app::colors::ColorScheme::default();
        let area = Rect::new(0, 0, 40, 24);
        let mut buf = Buffer::empty(area);

        let meta = minimal_story_meta(); // helper defined below

        super::draw_info_panel(
            "Cover Test", "cover-test.gblorb", &meta, None,
            0, area, Some(&picker), &mut cover, &path, false, &cs, &mut buf,
        );

        // Half-blocks emit the upper-half-block glyph in the reserved top band.
        // Collect the columns holding image pixels.
        let band_rows = area.top()..area.top() + area.height / 2;
        let img_cols: Vec<u16> = (area.left()..area.right())
            .filter(|&x| {
                band_rows
                    .clone()
                    .any(|y| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}"))
            })
            .collect();
        assert!(!img_cols.is_empty(), "cover band should contain half-block pixels");

        // The fitted (square) cover is CENTERED within the band, not left-aligned:
        // there is letterbox margin on both sides. Panel border is at x=0, so the
        // band's inner content starts at x=1.
        let min_x = *img_cols.iter().min().unwrap();
        let max_x = *img_cols.iter().max().unwrap();
        assert!(min_x > 1, "cover should have a left letterbox margin (leftmost col = {min_x})");
        assert!(
            max_x < area.right() - 2,
            "cover should have a right letterbox margin (rightmost col = {max_x})"
        );

        // The band is now sized to the image's aspect-fitted height (`used_h`),
        // not a fixed half-panel box: the info text should begin immediately
        // under the image, with no dead letterbox rows pushing it down.
        let last_image_row = band_rows
            .clone()
            .filter(|&y| {
                (area.left()..area.right())
                    .any(|x| buf.cell((x, y)).map(|c| c.symbol()) == Some("\u{2580}"))
            })
            .max()
            .expect("cover band should contain at least one image row");
        let title_row = (area.top()..area.bottom())
            .find(|&y| {
                let row_text = (area.left()..area.right())
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>();
                row_text.contains("Cover Test")
            })
            .expect("title text should appear in the panel");
        assert_eq!(
            title_row,
            last_image_row + 1,
            "title should begin immediately under the fitted image, no dead letterbox rows \
             (last image row = {last_image_row}, title row = {title_row})"
        );
    }

    // ── Story-picker info panel: toggle/slide/split ─────────────────────────────

    #[test]
    fn slide_fraction_interpolates_and_reverses() {
        // A closed→open slide at t=0 is 0.0, at t=1 is 1.0; reversing mid-slide
        // starts from the current fraction.
        let mut s = super::PanelSlide::closed();
        assert_eq!(s.fraction_at(0.0), 0.0);
        s.toggle_to(true, /*instant=*/true);
        assert_eq!(s.fraction_at(1.0), 1.0);
        s.toggle_to(false, true);
        assert_eq!(s.fraction_at(1.0), 0.0);
    }

    #[test]
    fn panel_refuses_to_open_when_too_narrow() {
        // Below LIST_MIN_W + PANEL_MIN_W the toggle is a no-op.
        assert!(!super::can_open_panel(super::LIST_MIN_W + super::PANEL_MIN_W - 1));
        assert!(super::can_open_panel(super::LIST_MIN_W + super::PANEL_MIN_W));
    }

    #[test]
    fn draw_story_picker_full_width_then_split() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        let stories = make_two_test_stories();
        let badges = vec![app::picker::RowBadges::default(); 2];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(2);

        // Closed: list uses full width, no panel border cell on the right edge.
        let area = Rect::new(0, 0, 70, 12);
        let mut buf = Buffer::empty(area);
        let (list_area, panel_area) = super::split_picker_area(area, 0.0);
        assert_eq!(list_area.width, area.width);
        assert_eq!(panel_area.width, 0);

        // Open (fraction 1.0): list shrinks, a panel area with width >= PANEL_MIN_W appears.
        let (list_area, panel_area) = super::split_picker_area(area, 1.0);
        assert!(list_area.width < area.width);
        assert!(panel_area.width >= super::PANEL_MIN_W);
        let _ = (&stories, &badges, &glyphs, &cs, &mut buf, &mut list);
    }
}
