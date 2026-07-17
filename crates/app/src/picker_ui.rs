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

/// Which story-picker view is active. `List` is the metadata table (default);
/// `Gallery` is the cover-thumbnail grid (SQ-0374). Toggled with `g`; the info
/// panel toggles independently (`i`/`Tab`) in both views.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerView {
    List,
    Gallery,
}

/// A previewable bundled resource the info panel links to (SQ-0347): an image
/// (`Pict`) or a sound (`Snd `). Carries where to re-read the bytes from (the
/// story's own blorb, or its sidecar) since the panel's `ChunkInfo` list holds
/// only display strings, not the resource data.
#[derive(Clone)]
struct ResourceRef {
    blorb_path: std::path::PathBuf,
    kind: PreviewKind,
    number: u32,
    label: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PreviewKind {
    Image,
    Sound,
}

/// A bundled resource being shown in the picker's preview modal (SQ-0347): a
/// decoded image (rebuilt to a fitted protocol as the modal draws) and/or a
/// one-line status (sound playback result, or why an image can't be shown).
struct ResourcePreview {
    /// Dialog title, e.g. `"Image #7"`.
    title: String,
    /// The decoded image, when this is a renderable Pict.
    image: Option<image::DynamicImage>,
    /// Fitted protocol cached by the content rect `(w, h)` it was built for.
    proto: Option<(u16, u16, ratatui_image::protocol::Protocol)>,
    /// A status line shown instead of (or below) an image.
    status: Option<String>,
}

/// Story-list row layout: the selection-marker glyph column, the gap between
/// text columns, and each data column's target width. Year drops first as
/// the row narrows, then author, leaving title + badges at the narrowest —
/// see `compute_columns`.
const ROW_MARKER_W: u16 = 2;
const COL_GAP: u16 = 2;
const AUTHOR_COL_W: u16 = 20;
const AUTHOR_MAX_W: u16 = 40;
const YEAR_COL_W: u16 = 6;
/// Interpreter/format column ("Z5", "Z5 (blorb)", "G3.1.2"): fixed width, sits
/// just left of the badge cluster. `Z8 (blorb)` (10) is the widest (SQ-0369).
const INTERP_COL_W: u16 = 10;
const TITLE_MIN_W: u16 = 8;
/// Title keeps this much before the author column is allowed to grow past its
/// base width — title has priority for the shared space, so a long author name
/// never squeezes the title down to `TITLE_MIN_W`.
const TITLE_PREFERRED_W: u16 = 24;

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

/// `want_author_w` is the widest author display width to show in full; the
/// author column grows from `AUTHOR_COL_W` toward it, but never so far that
/// title would drop below `TITLE_MIN_W`, and never past `AUTHOR_MAX_W` so one
/// very long name can't swallow the row. Title still absorbs any leftover, so
/// there is no gap before the badges.
fn compute_columns(text_w: u16, want_author_w: u16) -> ListColumns {
    let avail = text_w.saturating_sub(ROW_MARKER_W);
    let need_year = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W + COL_GAP + YEAR_COL_W;
    let need_author = TITLE_MIN_W + COL_GAP + AUTHOR_COL_W;
    let grow = |cols_space: u16| -> u16 {
        // Space shared by title+author (year already excluded). Author gets at
        // least AUTHOR_COL_W and grows toward want_author_w, but only into space
        // left after title keeps TITLE_PREFERRED_W — title has priority, so a
        // long name can't shrink the title below a comfortable width. Capped at
        // AUTHOR_MAX_W so one very long name can't swallow the row either.
        let ceiling = AUTHOR_MAX_W.min(cols_space.saturating_sub(TITLE_PREFERRED_W));
        want_author_w.clamp(AUTHOR_COL_W, ceiling.max(AUTHOR_COL_W))
    };
    if avail >= need_year {
        let cols_space = avail - COL_GAP - COL_GAP - YEAR_COL_W;
        let author_w = grow(cols_space);
        ListColumns { title_w: cols_space - author_w, author_w, year_w: YEAR_COL_W }
    } else if avail >= need_author {
        let cols_space = avail - COL_GAP;
        let author_w = grow(cols_space);
        ListColumns { title_w: cols_space - author_w, author_w, year_w: 0 }
    } else {
        ListColumns { title_w: avail, author_w: 0, year_w: 0 }
    }
}

/// Short interpreter/format label for the story-list TYPE column, type letter
/// plus the detected VM version: `Z<v>` for Z-code ("Z5", "Z3") and `G<v>` for
/// Glulx ("G3.1.2", from the Glulx header version). A blorb-wrapped Z-machine
/// story gets a " (blorb)" suffix ("Z5 (blorb)"), which subsumes the old B
/// badge; Glulx is omitted since Glulx games are effectively always blorbed
/// (SQ-0369). Bare "Z"/"Glulx" when the version is unknown.
fn interp_label(meta: &app::picker::StoryMeta, blorb: bool) -> String {
    match meta.engine {
        app::picker::Engine::ZCode => {
            let base = match meta.version.as_deref() {
                Some(v) if !v.is_empty() => format!("Z{v}"),
                _ => "Z".to_string(),
            };
            if blorb {
                format!("{base} (blorb)")
            } else {
                base
            }
        }
        app::picker::Engine::Glulx => match meta.version.as_deref() {
            Some(v) if !v.is_empty() => format!("G{v}"),
            _ => "Glulx".to_string(),
        },
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

/// Word-wrap `s` to at most `width` display columns per line (unicode-aware,
/// same width rule as `truncate_to_width`), splitting greedily on whitespace.
/// A blank line in `s` (a paragraph break) is preserved as an empty output
/// line. A single word wider than `width` is placed on its own line rather
/// than broken mid-word — same as any other overlong field, it is left for
/// the renderer to clip. `width == 0` returns `s` verbatim as one line.
fn wrap_to_width(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut lines = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for word in para.split_whitespace() {
            let word_w = UnicodeWidthStr::width(word);
            let sep_w = if cur.is_empty() { 0 } else { 1 };
            if !cur.is_empty() && cur_w + sep_w + word_w > width {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            if !cur.is_empty() {
                cur.push(' ');
                cur_w += 1;
            }
            cur.push_str(word);
            cur_w += word_w;
        }
        lines.push(cur);
    }
    lines
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
const FOOTER_OPTIONAL: [&str; 7] =
    ["g: covers", "f: fetch", "r: refresh", "u: IFDB url", "s: sort", "d: reverse", "PgUp/PgDn"];

fn build_footer(width: u16) -> String {
    const CORE_LEFT: &str = " ↑/↓ or j/k: move";
    const CORE_RIGHT: &str = "Enter / 2×click: open   i/Tab: info   q / Esc: quit";
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

/// Reorder `stories` by `sort`, keeping the selection on the same story (by
/// path — see `resort_preserving_selection`), and keep the per-index caches
/// (`row_badges`, `aux_cache`) aligned with the new order. Every reorder in
/// the picker loop — `s`, `d`, a header click, and a fetch sweep landing new
/// titles — routes through this one function so no caller can forget to
/// invalidate the caches.
#[allow(clippy::too_many_arguments)]
fn resort_list(
    stories: &mut [app::picker::StoryEntry],
    selected: usize,
    sort: app::picker::Sort,
    row_badges: &mut Vec<app::picker::RowBadges>,
    aux_cache: &mut Vec<Option<app::picker::StoryAux>>,
    data_base: &std::path::Path,
    hint_index: &app::hints::HintIndex,
) -> usize {
    let new_idx = app::picker::resort_preserving_selection(stories, selected, sort);
    *row_badges = stories
        .iter()
        .map(|e| app::picker::compute_row_badges(e, data_base, hint_index))
        .collect();
    *aux_cache = (0..stories.len()).map(|_| None).collect();
    new_idx
}

/// Overlay a transient status line (fetch progress) onto the list's footer
/// row, replacing the normal footer hint text while a message is active.
fn draw_progress_line(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    text: &str,
    style: ratatui::style::Style,
) {
    if area.height < 4 {
        return; // matches draw_story_picker's own too-small-for-a-footer guard
    }
    let y = area.bottom().saturating_sub(1);
    for x in area.left()..area.right() {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol(" ").set_style(style);
        }
    }
    draw_str_clipped(buf, area.x, y, text, style, area);
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
    let mut stories = app::picker::scan_stories(dir, data_base);
    if stories.is_empty() {
        eprintln!("babelmap: no Z-machine story files found in '{}'", dir.display());
        std::process::exit(1);
    }

    // Resolve themed colors the same way the game does, so the picker matches.
    let (base, _w1) = app::style::load_style(cfg.style.as_deref(), &cfg.user_dir);
    let (cs, _set, _w2) = app::style::resolve(&base, &cfg.user_dir);

    // Row badges: each story's per-game dir under `data_base` + one shared hint
    // index, computed once (SQ-0284). Recomputed by `resort_list` whenever the
    // list reorders, so it stays index-aligned with `stories`.
    let hint_index = app::hints::load_hint_index(&cfg.user_dir);
    let mut row_badges: Vec<app::picker::RowBadges> = stories
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
    let mut sort = app::picker::Sort::default();

    // Cover-gallery view state (SQ-0374): `view` selects list-vs-grid; the rest
    // is grid geometry from the last gallery draw, read by input handling
    // (2D navigation, paging) and the per-frame visible-tile cover requests.
    let mut view = PickerView::List;
    let mut gallery_first_row: usize = 0;
    let mut gallery_cols: usize = 1;
    let mut gallery_vis: usize = 1;
    let mut gallery_visible: Vec<usize> = Vec::new();

    // IFDB fetch worker (SQ-0348): `f` (this story, forced) and `r` (whole
    // library, skip current-version) share one background worker. Live only
    // while this loop runs — dropping `fetcher` at the end drops its request
    // sender, which ends the worker thread's `recv()` loop.
    let fetcher = app::fetch_worker::Fetcher::new(
        Box::new(app::ifdb::IfdbClient::new()),
        data_base.to_path_buf(),
        Duration::from_millis(500),
    );
    // The footer-row status line while a fetch is in flight (or just finished);
    // `None` shows the normal footer hints instead.
    let mut progress_line: Option<String> = None;
    // True for an `f` order (single story, forced) — controls the completion
    // message's shape (`f`: found/not-found/failed; `r`: a tallied summary).
    let mut fetch_is_single = false;
    let (mut sweep_fetched, mut sweep_skipped, mut sweep_not_found, mut sweep_failed) = (0u32, 0u32, 0u32, 0u32);
    // Manual IFDB-page entry (SQ-0371): `Some` while the user is typing an IFDB
    // URL/id for the selected story; keystrokes route to the field, Enter
    // submits a fetch-by-id, Esc cancels.
    let mut manual_ifdb: Option<app::text_field::TextField> = None;

    // Info panel: always starts closed each launch (session-only state).
    let mut slide = PanelSlide::closed();
    let mut aux_cache: Vec<Option<app::picker::StoryAux>> =
        (0..stories.len()).map(|_| None).collect();
    let mut last_area = Rect::new(0, 0, 0, 0);
    let mut last_panel_area = Rect::new(0, 0, 0, 0);
    let mut panel_scroll: usize = 0;
    let mut panel_max: usize = 0;
    // Screen rect + full URL of each OSC 8 link the info panel drew this frame,
    // so a click can open it despite mouse capture (SQ-0367). Empty while the
    // panel is closed (refilled only when draw_info_panel runs).
    let mut panel_link_rects: Vec<(Rect, String)> = Vec::new();
    // Screen rect + resource ref of each previewable Pict/Snd row drawn this
    // frame (SQ-0347), so a click opens its preview modal.
    let mut panel_resource_rects: Vec<(Rect, ResourceRef)> = Vec::new();

    // Resource-preview modal (SQ-0347): `Some` while a bundled image/sound is
    // shown over the picker. `audio` is constructed lazily on the first sound
    // play and held for the loop's lifetime (its OutputStream must outlive
    // playback; per-click construct/drop would cut the sound and stutter).
    let mut preview: Option<ResourcePreview> = None;
    let mut audio: Option<audio::AudioBackend> = None;
    let mut preview_close_rect: Option<Rect> = None;
    let mut preview_button_rects: Vec<(app::render::dialog::ButtonId, Rect)> = Vec::new();
    let mut preview_area = Rect::new(0, 0, 0, 0);

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
    // Story-list clicks: first click selects, a second on the same row within
    // this window launches it (SQ-0366).
    let mut last_click: Option<(usize, Instant)> = None;
    const DOUBLE_CLICK: Duration = Duration::from_millis(400);
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
                if matches!(view, PickerView::Gallery) {
                    // One notch = one grid row (a whole row of tiles).
                    let ni = app::cover_gallery::move_index(
                        list.selected, gallery_cols, stories.len(), 0, d,
                    );
                    list.select(ni, viewport, anim);
                } else {
                    list.move_by(d, viewport, anim);
                }
            }
        }

        let _ = terminal.draw(|f| {
            let area = f.area();
            last_area = area;
            let buf = f.buffer_mut();
            let (list_area, panel_area) = split_picker_area(area, slide.fraction());
            match view {
                PickerView::List => {
                    let (rects, vp, hrects) = draw_story_picker(
                        &stories, &list, &row_badges, &badge_glyphs, dir, &cs,
                        sort, list_area, buf,
                    );
                    row_rects = rects;
                    viewport = vp;
                    header_rects = hrects;
                }
                PickerView::Gallery => {
                    let (rects, cols, vis) = draw_story_gallery(
                        &stories, list.selected, &mut gallery_first_row, dir, &cs,
                        cover_picker.as_ref(), &mut cover, list_area, buf,
                    );
                    gallery_cols = cols.max(1);
                    gallery_vis = vis.max(1);
                    gallery_visible = rects.iter().map(|(i, _)| *i).collect();
                    // A viewport analogue for ListScroll's easing while it holds
                    // the shared selection; the grid does its own scrolling.
                    viewport = (cols * vis).max(1);
                    row_rects = rects;
                    header_rects = Vec::new();
                }
            }
            // The manual IFDB-entry prompt (SQ-0371) takes the footer row while
            // active; otherwise a fetch's status line, otherwise the hints.
            if let Some(field) = &manual_ifdb {
                let prompt = format!("IFDB URL or id (Enter to fetch, Esc to cancel): {}▏", field.as_str());
                draw_progress_line(buf, list_area, &prompt, cs.story_header_active);
            } else if let Some(msg) = &progress_line {
                draw_progress_line(buf, list_area, msg, cs.story_header_active);
            }
            if preview.is_none() && panel_area.width > 0 {
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
                        &mut panel_link_rects,
                        &mut panel_resource_rects,
                    );
                } else {
                    panel_link_rects.clear();
                    panel_resource_rects.clear();
                }
            } else {
                // No selectable story, or the resource-preview modal is open: skip
                // redrawing the info panel. While the modal is up this stops the
                // panel's IFDB OSC-8 hyperlink from bleeding across the dialog
                // (SQ-0389), and drops its now-hidden click rects.
                panel_link_rects.clear();
                panel_resource_rects.clear();
            }

            // Resource-preview modal (SQ-0347): drawn last, over everything.
            if let Some(pv) = &mut preview {
                let rects = draw_resource_preview(pv, area, cover_picker.as_ref(), &cs, buf);
                preview_area = rects.area;
                preview_close_rect = rects.close;
                preview_button_rects = rects.buttons;
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
                let game_dir = app::storage::game_dir(data_base, &app::storage::story_key(&sel));
                decoder.request(sel.clone(), game_dir);
                requested.insert(sel);
            }
        }
        // Gallery view (SQ-0374): decode every visible tile's cover, not just the
        // selection — the whole grid shows art. No settle-debounce here: the user
        // wants them all, and the worker decodes them one at a time as it can.
        if view == PickerView::Gallery {
            for &idx in &gallery_visible {
                if let Some(entry) = stories.get(idx) {
                    let p = entry.path.clone();
                    if !cover.has(&p) && !requested.contains(&p) {
                        let game_dir = app::storage::game_dir(data_base, &app::storage::story_key(&p));
                        decoder.request(p.clone(), game_dir);
                        requested.insert(p);
                    }
                }
            }
        }

        // Drain fetch progress (SQ-0348): each completed story's sidecar may
        // have just been (re)written, so re-resolve its entry in place — same
        // path both `f` and `r` take, since a single-story order is just an
        // order of length one — then re-sort through the one shared helper so
        // the cursor stays on whatever story the user is actually looking at,
        // not wherever its index happened to land.
        let mut fetch_arrived = false;
        for p in fetcher.drain() {
            fetch_arrived = true;
            match &p.outcome {
                app::fetch_worker::Outcome::Fetched => sweep_fetched += 1,
                app::fetch_worker::Outcome::Skipped => sweep_skipped += 1,
                app::fetch_worker::Outcome::NotFound => sweep_not_found += 1,
                app::fetch_worker::Outcome::Failed(_) => sweep_failed += 1,
            }
            // Only Fetched/NotFound actually (re)write the sidecar (a Skipped
            // story's cache was already current; a Failed story's write is
            // withheld so a later `r` retries it) — no point re-reading disk
            // for the other two.
            let rewrote_sidecar =
                matches!(p.outcome, app::fetch_worker::Outcome::Fetched | app::fetch_worker::Outcome::NotFound);
            if rewrote_sidecar {
                if let Some(fresh) = app::picker::resolve_entry(&p.path, data_base) {
                    if let Some(slot) = stories.iter_mut().find(|e| e.path == p.path) {
                        *slot = fresh;
                    }
                }
                // A fetch may have just written a cover.png; drop any cached
                // "coverless" decode so the panel re-reads and shows it now,
                // rather than only after the picker is reopened.
                if matches!(p.outcome, app::fetch_worker::Outcome::Fetched) {
                    cover.forget(&p.path);
                    requested.remove(&p.path);
                }
            }
            progress_line = Some(if fetch_is_single {
                match &p.outcome {
                    app::fetch_worker::Outcome::Fetched => format!("Fetched {}", p.title),
                    app::fetch_worker::Outcome::Skipped => format!("Fetched {}", p.title),
                    app::fetch_worker::Outcome::NotFound => format!("No IFDB record for {}", p.title),
                    app::fetch_worker::Outcome::Failed(reason) => format!("Fetch failed: {reason}"),
                }
            } else if p.done < p.total {
                format!("Fetching {}/{} — {}", p.done, p.total, p.title)
            } else {
                let mut msg = format!(
                    "Fetched {}, skipped {}, not found {}",
                    sweep_fetched, sweep_skipped, sweep_not_found
                );
                if sweep_failed > 0 {
                    msg.push_str(&format!(", failed {sweep_failed}"));
                }
                msg
            });
        }
        if fetch_arrived {
            list.select(
                resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                viewport,
                anim,
            );
        }

        // A decode just landed: loop back to redraw so the cover paints now. The
        // draw is at the top of the loop, and once the result is cached
        // `cover_busy` goes false — without this the loop would block on `read()`
        // and the new cover wouldn't appear until the next input event.
        if cover_arrived || fetch_arrived {
            list.finalize_if_done();
            continue;
        }

        // Tick while a scroll or panel-slide animation eases so the motion is
        // visible, or while a cover decode is in flight / still needed, or a
        // fetch sweep is running, so results drain and redraw without a
        // keypress; otherwise block until the next event.
        let sel_now = stories.get(list.selected).map(|e| &e.path);
        let panel_busy = slide.open
            && sel_now.is_some_and(|p| !requested.is_empty() || !cover.has(p));
        // Gallery keeps ticking while any tile cover is still decoding so the
        // grid fills in without needing a keypress.
        let gallery_busy = matches!(view, PickerView::Gallery) && !requested.is_empty();
        let cover_busy = panel_busy || gallery_busy;
        if (list.has_active_animation() || slide.active() || cover_busy || fetcher.busy())
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
                // The resource-preview modal (SQ-0347) captures all keys while
                // open: any of Esc/Enter/q/Space dismisses it (and stops a sound).
                if preview.is_some() {
                    if matches!(k.code, Esc | Enter | Char('q') | Char(' ')) {
                        preview = None;
                        if let Some(a) = audio.as_mut() {
                            a.stop_all();
                        }
                    }
                } else if let Some(field) = manual_ifdb.as_mut() {
                    match k.code {
                        Esc => {
                            manual_ifdb = None;
                            progress_line = None;
                        }
                        Enter => {
                            let input = field.take();
                            manual_ifdb = None;
                            if let Some(entry) = stories.get(list.selected) {
                                match app::ifdb::extract_tuid(&input) {
                                    Some(tuid) => {
                                        fetch_is_single = true;
                                        sweep_fetched = 0;
                                        sweep_skipped = 0;
                                        sweep_not_found = 0;
                                        sweep_failed = 0;
                                        progress_line = Some(format!("Fetching {} from IFDB…", entry.title));
                                        fetcher.request(app::fetch_worker::FetchOrder {
                                            stories: vec![(entry.path.clone(), entry.meta.ifid.clone())],
                                            forced: true,
                                            id_override: Some(tuid),
                                        });
                                    }
                                    None => {
                                        progress_line = Some("Not an IFDB URL or id".to_string());
                                    }
                                }
                            }
                        }
                        Backspace => field.backspace(),
                        Delete => field.delete(),
                        Left => field.left(),
                        Right => field.right(),
                        Home => field.home(),
                        End => field.end(),
                        Char(c) => field.insert(c),
                        _ => {}
                    }
                } else if slide.open && shift {
                    let page = (last_panel_area.height.saturating_sub(2)).max(1) as usize;
                    match k.code {
                        Up => panel_scroll = panel_scroll.saturating_sub(1),
                        Down => panel_scroll = (panel_scroll + 1).min(panel_max),
                        PageUp => panel_scroll = panel_scroll.saturating_sub(page),
                        PageDown => panel_scroll = (panel_scroll + page).min(panel_max),
                        _ => {}
                    }
                } else {
                    // A finished fetch leaves its summary on the footer row; the
                    // next keypress (once nothing is in flight) clears it so the
                    // normal hints return — otherwise the summary would sit there
                    // for the rest of the session, hiding the key legend.
                    if !fetcher.busy() {
                        progress_line = None;
                    }
                    // Gallery navigation moves a 2D cursor over the same shared
                    // selection; the list moves it linearly. `gm` computes the
                    // clamped grid target for a (dx, dy) step.
                    let gm = |sel: usize, dx: isize, dy: isize| {
                        app::cover_gallery::move_index(sel, gallery_cols, stories.len(), dx, dy)
                    };
                    let gallery = matches!(view, PickerView::Gallery);
                    match k.code {
                        // Horizontal movement exists only in the grid.
                        Left | Char('h') if gallery => {
                            panel_scroll = 0;
                            list.select(gm(list.selected, -1, 0), viewport, anim);
                        }
                        Right | Char('l') if gallery => {
                            panel_scroll = 0;
                            list.select(gm(list.selected, 1, 0), viewport, anim);
                        }
                        Up | Char('k') => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(gm(list.selected, 0, -1), viewport, anim);
                            } else {
                                list.move_by(-1, viewport, anim);
                            }
                        }
                        Down | Char('j') => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(gm(list.selected, 0, 1), viewport, anim);
                            } else {
                                list.move_by(1, viewport, anim);
                            }
                        }
                        PageUp => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(gm(list.selected, 0, -(gallery_vis as isize)), viewport, anim);
                            } else {
                                list.page(-1, viewport, anim);
                            }
                        }
                        PageDown => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(gm(list.selected, 0, gallery_vis as isize), viewport, anim);
                            } else {
                                list.page(1, viewport, anim);
                            }
                        }
                        Home => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(0, viewport, anim);
                            } else {
                                list.home(viewport, anim);
                            }
                        }
                        End => {
                            panel_scroll = 0;
                            if gallery {
                                list.select(stories.len().saturating_sub(1), viewport, anim);
                            } else {
                                list.end(stories.len(), viewport, anim);
                            }
                        }
                        Enter => break Some(stories[list.selected].path.clone()),
                        Char('q') => break None,
                        // Esc cancels a running sweep first; only quits when
                        // nothing is in flight.
                        Esc => {
                            if fetcher.busy() {
                                fetcher.cancel();
                            } else {
                                break None;
                            }
                        }
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
                        // `g`: toggle the cover-gallery grid (SQ-0374). Selection
                        // carries over; reset the grid scroll so the selected
                        // cover is framed on entry (the next draw scrolls to it).
                        Char('g') => {
                            view = match view {
                                PickerView::List => PickerView::Gallery,
                                PickerView::Gallery => PickerView::List,
                            };
                            gallery_first_row = 0;
                        }
                        // `f`: refetch only the selected story, ignoring its
                        // cache. Ignored while a sweep is already running, so a
                        // second press can't garble the in-flight progress line.
                        Char('f') if !fetcher.busy() => {
                            if let Some(entry) = stories.get(list.selected) {
                                fetch_is_single = true;
                                sweep_fetched = 0;
                                sweep_skipped = 0;
                                sweep_not_found = 0;
                                sweep_failed = 0;
                                progress_line = Some(format!("Fetching {}…", entry.title));
                                fetcher.request(app::fetch_worker::FetchOrder {
                                    stories: vec![(entry.path.clone(), entry.meta.ifid.clone())],
                                    forced: true,
                                    id_override: None,
                                });
                            }
                        }
                        // `r`: sweep the whole library; the worker itself skips
                        // any story already at the current FETCH_VERSION. Ignored
                        // while a sweep is already running (see `f`).
                        Char('r') if !fetcher.busy() => {
                            let total = stories.len();
                            let order: Vec<(PathBuf, String)> =
                                stories.iter().map(|e| (e.path.clone(), e.meta.ifid.clone())).collect();
                            fetch_is_single = false;
                            sweep_fetched = 0;
                            sweep_skipped = 0;
                            sweep_not_found = 0;
                            sweep_failed = 0;
                            progress_line = Some(format!("Fetching 0/{total}"));
                            fetcher.request(app::fetch_worker::FetchOrder { stories: order, forced: false, id_override: None });
                        }
                        // `u`: point the selected story at an IFDB page by hand
                        // (for a story whose IFID IFDB doesn't index). Opens the
                        // manual-entry field; ignored mid-sweep (SQ-0371).
                        Char('u') if !fetcher.busy() => {
                            manual_ifdb = Some(app::text_field::TextField::new(""));
                            progress_line = None;
                        }
                        // `s`: cycle the sort column, keeping direction. `d`:
                        // toggle direction, keeping the column. Both preserve
                        // the selection by path, never by index.
                        Char('s') => {
                            sort.key = match sort.key {
                                app::picker::SortKey::Title => app::picker::SortKey::Author,
                                app::picker::SortKey::Author => app::picker::SortKey::Year,
                                app::picker::SortKey::Year => app::picker::SortKey::Title,
                            };
                            list.select(
                                resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                                viewport,
                                anim,
                            );
                        }
                        Char('d') => {
                            sort.desc = !sort.desc;
                            list.select(
                                resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                                viewport,
                                anim,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::Mouse(m)) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                if let MouseEventKind::Down(MouseButton::Left) = m.kind {
                    let pt = ratatui::layout::Position { x: m.column, y: m.row };
                    if preview.is_some() {
                        // Modal open (SQ-0347): the ✕, the Close button, or a click
                        // outside the dialog all dismiss it (and stop a sound); a
                        // click inside is swallowed.
                        let on_close = preview_close_rect.is_some_and(|r| r.contains(pt));
                        let on_button = preview_button_rects.iter().any(|(_, r)| r.contains(pt));
                        let outside = !preview_area.contains(pt);
                        if on_close || on_button || outside {
                            preview = None;
                            if let Some(a) = audio.as_mut() {
                                a.stop_all();
                            }
                        }
                    } else if let Some((_, rref)) = panel_resource_rects.iter().find(|(r, _)| r.contains(pt)) {
                        // Click on a previewable Pict/Snd resource row (SQ-0347):
                        // open its modal (image renders / sound plays).
                        preview = Some(open_resource_preview(rref, &mut audio, cfg.volume));
                    } else if let Some((_, url)) = panel_link_rects.iter().find(|(r, _)| r.contains(pt)) {
                        // Click on an info-panel OSC 8 link (SQ-0367): the terminal
                        // can't act on it while we hold mouse capture, so open it.
                        open_url(url);
                    } else if let Some((idx, _)) = row_rects.iter().find(|(_, r)| r.contains(pt)) {
                        let idx = *idx;
                        let now = Instant::now();
                        // Second click on the already-selected row within the
                        // window → launch; otherwise just select it (SQ-0366).
                        let double = last_click
                            .is_some_and(|(li, lt)| li == idx && now.duration_since(lt) < DOUBLE_CLICK);
                        if double {
                            break Some(stories[idx].path.clone());
                        }
                        panel_scroll = 0;
                        list.select(idx, viewport, anim);
                        if slide.open {
                            ensure_aux(&mut aux_cache, &stories, list.selected, data_base, &hint_index);
                        }
                        last_click = Some((idx, now));
                    } else if let Some((key, _)) = header_rects.iter().find(|(_, r)| r.contains(pt)) {
                        // Click the active header → reverse; click another → sort
                        // by it, ascending.
                        if sort.key == *key {
                            sort.desc = !sort.desc;
                        } else {
                            sort.key = *key;
                            sort.desc = false;
                        }
                        list.select(
                            resort_list(&mut stories, list.selected, sort, &mut row_badges, &mut aux_cache, data_base, &hint_index),
                            viewport,
                            anim,
                        );
                    }
                } else if let Some(d) = app::input::wheel_delta(m.kind, cfg.mouse_wheel_invert) {
                    // The preview modal swallows the wheel so the list behind it
                    // doesn't scroll (SQ-0347).
                    if preview.is_some() {
                        // no-op while the modal is open
                    } else {
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

/// Per-row hit-rects (row index, rect) for mouse selection.
type RowHitRects = Vec<(usize, Rect)>;
/// Column-header hit-rects (sort key, rect) for click-to-sort.
type HeaderHitRects = Vec<(app::picker::SortKey, Rect)>;

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
) -> (RowHitRects, usize, HeaderHitRects) {
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
        " babelmap — choose a story  ({} found in {})   [i: info · g: covers]",
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
    // and to place each row's cluster. Both the interpreter/format AND the
    // blorb indicator moved into the TYPE column (SQ-0369: "Z5 (blorb)"), so
    // the cluster is now just [save][hint].
    let save_w = glyphs.save.chars().count() as u16;
    let hint_w = glyphs.hint.chars().count() as u16;
    let cluster_w = save_w + hint_w;
    // The TYPE column rides in the same right-hand zone as the badges, one gap
    // to their left; both are shown together or dropped together.
    let right_zone = INTERP_COL_W + COL_GAP + cluster_w;
    let badges_shown = right_zone + 2 < row_w;
    let badge_reserved = if badges_shown { right_zone + 1 } else { 0 };
    let text_w = row_w.saturating_sub(badge_reserved);
    // Widest author name across the WHOLE list (not just the visible page), so
    // the author column doesn't jump width as the user scrolls.
    let want_author_w = stories
        .iter()
        .filter_map(|e| e.meta.author.as_deref())
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0) as u16;
    let cols = compute_columns(text_w, want_author_w);

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
    // TYPE column header, above the interpreter labels in the right-hand zone.
    // Not sortable, so no header rect — just a dimmed label (SQ-0369).
    if badges_shown {
        let interp_hx = area.left() + row_w - 1 - cluster_w - COL_GAP - INTERP_COL_W;
        draw_str_clipped(buf, interp_hx, header_y, "TYPE", cs.story_header, area);
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

        // Right-hand zone: the TYPE column then the badge cluster
        // [blorb][save][hint]. No separators within the cluster, so present
        // badges stay vertically aligned across rows.
        let b = badges.get(i).copied().unwrap_or_default();
        if badges_shown {
            let bx = area.left() + row_w - 1 - cluster_w;
            // TYPE column, one gap to the left of the badges. Plain colour (not
            // the badge's reverse-block treatment); selection wins like the
            // other text columns.
            let interp_x = bx - COL_GAP - INTERP_COL_W;
            let interp_txt =
                truncate_to_width(&interp_label(&entry.meta, b.blorb), INTERP_COL_W as usize);
            let interp_style = if sel { style } else { cs.story_badge };
            draw_str_clipped(buf, interp_x, y, &interp_txt, interp_style, row_rect);
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
            if b.save {
                draw_str_clipped(buf, bx, y, glyphs.save, badge_style, row_rect);
            }
            if b.hint {
                draw_str_clipped(buf, bx + save_w, y, glyphs.hint, badge_style, row_rect);
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

/// Draw the cover-gallery view (SQ-0374): a grid of story cover thumbnails, each
/// a fitted frontispiece over a one-row title caption, with the selected tile's
/// caption highlighted. `first_row` is the grid's scroll row (in/out — updated
/// to keep `selected` on screen). Returns each visible tile's `(index, rect)`
/// for click selection (the whole tile is the hit target), plus the resolved
/// column and visible-row counts the caller feeds back into navigation. Covers
/// paint only for tiles already decoded into `cover`; undecoded/coverless tiles
/// show a plain letterbox until the async decoder fills them in.
#[allow(clippy::too_many_arguments)]
fn draw_story_gallery(
    stories: &[app::picker::StoryEntry],
    selected: usize,
    first_row: &mut usize,
    dir: &std::path::Path,
    cs: &app::colors::ColorScheme,
    picker: Option<&ratatui_image::picker::Picker>,
    cover: &mut app::cover::CoverState,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) -> (Vec<(usize, Rect)>, usize, usize) {
    use app::cover_gallery as g;
    let mut tile_rects: Vec<(usize, Rect)> = Vec::new();

    // Background fill.
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(cs.dialog);
            }
        }
    }

    // Header (matches the list view's, with the toggle hint flipped).
    let header = format!(
        " babelmap — choose a story  ({} found in {})   [i: info · g: list]",
        stories.len(),
        dir.display()
    );
    draw_str_clipped(buf, area.x, area.y, &header, cs.dialog_title, area);

    // Grid region: below the header, above the footer row.
    let grid_top = area.y + 2;
    let grid_bottom = area.bottom().saturating_sub(1);
    if grid_bottom <= grid_top || area.width < g::TILE_W {
        return (tile_rects, 1, 1);
    }
    let grid = Rect::new(area.x, grid_top, area.width, grid_bottom - grid_top);
    let cols = g::columns(grid.width);
    let vis = g::visible_rows(grid.height);
    *first_row = g::scroll_to(selected, cols, vis, *first_row);
    let total = stories.len();
    let total_rows = total.div_ceil(cols);

    for vr in 0..vis {
        for col in 0..cols {
            let idx = (*first_row + vr) * cols + col;
            if idx >= total {
                continue;
            }
            let entry = &stories[idx];
            let tile = g::tile_rect(grid, col, vr);
            tile_rects.push((idx, tile));

            // Cover band = the whole tile. A selected tile's letterbox is tinted
            // with the selection style (the "glow" around the fitted image); an
            // unselected one uses the neutral cover fill. There is no title
            // caption — when a cover is missing, the title is centred in the band.
            let sel = idx == selected;
            let cover_rect = Rect::new(tile.x, tile.y, g::TILE_W, g::TILE_COVER_H);
            let fill_style = if sel { cs.story_tile_selected } else { cs.story_info_cover };
            for y in cover_rect.top()..cover_rect.bottom() {
                for x in cover_rect.left()..cover_rect.right() {
                    if let Some(c) = buf.cell_mut((x, y)) {
                        c.set_symbol(" ").set_style(fill_style);
                    }
                }
            }
            // Fit the image inside a small inset so the tile's fill shows as a
            // margin on ALL four sides: for a selected tile that margin is the
            // selection glow framing the whole cover, instead of only leaking out
            // the bottom when the fitted image is a hair shorter than the tile.
            // Wider inset on x than y so the frame reads evenly (cells are ~2:1).
            let inner = Rect::new(
                cover_rect.x + 2,
                cover_rect.y + 1,
                cover_rect.width.saturating_sub(4),
                cover_rect.height.saturating_sub(2),
            );
            let mut drew_cover = false;
            if let Some(picker) = picker {
                if cover.has(&entry.path) {
                    if let Some(proto) = cover.tile_protocol(picker, &entry.path, inner) {
                        let sz = proto.size();
                        let used_w = sz.width.min(inner.width);
                        let used_h = sz.height.min(inner.height);
                        let dest = Rect::new(
                            inner.x + (inner.width - used_w) / 2,
                            inner.y + (inner.height - used_h) / 2,
                            used_w,
                            used_h,
                        );
                        ratatui::widgets::Widget::render(
                            ratatui_image::Image::new(proto),
                            dest,
                            buf,
                        );
                        drew_cover = true;
                    }
                }
            }
            if !drew_cover {
                // No cover art: centre the wrapped title in the band, using the
                // selection style when selected so the tile still glows.
                let title_style = if sel { cs.story_tile_selected } else { cs.story_tile };
                for y in cover_rect.top()..cover_rect.bottom() {
                    for x in cover_rect.left()..cover_rect.right() {
                        if let Some(c) = buf.cell_mut((x, y)) {
                            c.set_symbol(" ").set_style(title_style);
                        }
                    }
                }
                let lines = wrap_to_width(&entry.title, g::TILE_W as usize);
                let shown = (lines.len() as u16).min(g::TILE_COVER_H);
                let start_y = cover_rect.y + (g::TILE_COVER_H - shown) / 2;
                for (i, line) in lines.iter().take(shown as usize).enumerate() {
                    let lw = UnicodeWidthStr::width(line.as_str()) as u16;
                    let x = cover_rect.x + g::TILE_W.saturating_sub(lw) / 2;
                    draw_str_clipped(buf, x, start_y + i as u16, line, title_style, cover_rect);
                }
            }
        }
    }

    // Scrollbar in the spare width to the grid's right when the grid overflows.
    if total_rows > vis {
        let sb_area = Rect::new(area.right().saturating_sub(1), grid.y, 1, grid.height);
        app::render::scroll::draw_scrollbar(buf, sb_area, total_rows, vis, *first_row, cs.scrollbar);
    }

    // Footer hint.
    let footer = " ←/→/↑/↓: move   Enter / 2×click: open   i/Tab: info   g: list   q / Esc: quit";
    let fstyle = ratatui::style::Style::new()
        .fg(ratatui::style::Color::DarkGray)
        .patch(cs.dialog);
    let footer_txt = truncate_to_width(footer, area.width as usize);
    draw_str_clipped(buf, area.x, grid_bottom, &footer_txt, fstyle, area);

    (tile_rects, cols, vis)
}

/// Open a URL in the user's default browser. The picker holds mouse capture
/// (for click-to-select), so the terminal never handles a plain click on an
/// OSC 8 hyperlink (SQ-0367) — we open it ourselves instead. Fire-and-forget:
/// the URL is passed as a single argument (no shell), so it needs no escaping.
fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = std::process::Command::new("xdg-open");

    let _ = cmd
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Draw the highlighted story's metadata panel: title, filesystem info,
/// format/version/release, serial (Z only), IFID, present features, bundled
/// resources (self-blorb or an associated sibling blorb), and saves. Pure
/// renderer — no state, no interaction (the picker loop wires toggling/
/// slide/lazy-resolve). `link_rects` is cleared and refilled with the screen
/// rect + full URL of every rendered OSC 8 link, so the loop can open one on a
/// click (mouse capture keeps the terminal from doing it — SQ-0367).
#[allow(clippy::too_many_arguments)]
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
    link_rects: &mut Vec<(Rect, String)>,
    resource_rects: &mut Vec<(Rect, ResourceRef)>,
) -> usize {
    link_rects.clear();
    resource_rects.clear();
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
    // Line index → full URL for lines that should render as OSC 8 hyperlinks
    // (SQ-0367): the visible text may truncate, but the whole visible label
    // stays clickable and opens the full URL.
    let mut link_urls: Vec<(usize, String)> = Vec::new();
    // Line index → previewable resource (SQ-0347), for the Pict/Snd rows below.
    let mut resource_refs: Vec<(usize, ResourceRef)> = Vec::new();

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
    // Associated resource blorb (SQ-0372): a resource .blorb stored beside the
    // story (e.g. Lurking.blb, beyondzork.blb). Named here, up-front, so it is
    // visible without scrolling to the Resources section below. Only the sidecar
    // case — a self-contained blorb's resources live in the story file itself.
    if let Some((src, _)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
        if let Some(name) = src.file_name().and_then(|n| n.to_str()) {
            lines.push((format!("Resource blorb: {name}"), cs.story_info_value));
        }
    }
    // author · year · genre (SQ-0348): one line, present parts only — a story
    // with none of the three renders no line at all, so a no-metadata panel
    // is unchanged from before this field existed.
    let meta_bits: Vec<&str> = [meta.author.as_deref(), meta.year.as_deref(), meta.genre.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    if !meta_bits.is_empty() {
        lines.push((meta_bits.join(" · "), cs.story_info_value));
    }
    // blurb (SQ-0348): word-wrapped to the panel's content width, each
    // wrapped row pushed as its own entry so it rides the same
    // scroll/overflow accounting as every other panel line below. Split on the
    // paragraph breaks html_to_text left in, wrapping each independently so a
    // `<br/>` in the source stays a visible break rather than collapsing.
    //
    // Reserve the scrollbar's gutter column when wrapping: whether the panel
    // overflows depends on the wrapped line count, so it can't be known here —
    // always wrap to the narrower width so a wrapped line is never clipped by
    // (or drawn under) the scrollbar in the overflow case.
    let wrap_w = (inner.width as usize).saturating_sub(1);
    if let Some(desc) = meta.description.as_deref().filter(|s| !s.is_empty()) {
        for para in desc.lines() {
            if para.trim().is_empty() {
                lines.push((String::new(), cs.story_info_blurb));
            } else {
                for row in wrap_to_width(para, wrap_w) {
                    lines.push((row, cs.story_info_blurb));
                }
            }
        }
    }
    // IFDB page link (SQ-0348): a real OSC 8 hyperlink (SQ-0367) so the visible
    // text is clickable even when the URL truncates; only present once fetched.
    if let Some(link) = meta.ifdb_link.as_deref().filter(|s| !s.is_empty()) {
        link_urls.push((lines.len(), link.to_string()));
        lines.push((format!("IFDB: {link}"), cs.story_info_link));
    } else if meta.fetch_not_found {
        // A fetch ran but IFDB had no record for this IFID (common for Infocom
        // releases IFDB indexes under a different IFID). Offer a manual search
        // by title so the user isn't at a dead end (SQ-0371).
        let url = app::ifdb::search_url(title);
        link_urls.push((lines.len(), url.clone()));
        lines.push((format!("IFDB search: {url}"), cs.story_info_link));
    }
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

    // Resources: self_blorb (the story is itself a blorb), else aux.assoc_blorb
    // (a sidecar). `blorb_path` is where a clicked resource re-reads its bytes.
    let (res_header, chunks, blorb_path): (Option<String>, &[app::picker::ChunkInfo], Option<std::path::PathBuf>) =
        if let Some(c) = &meta.self_blorb {
            (Some(format!("Resources ({filename})")), c.as_slice(), Some(entry_path.to_path_buf()))
        } else if let Some((p, c)) = aux.and_then(|a| a.assoc_blorb.as_ref()) {
            // The sidecar filename is named up-front in the metadata block, so
            // this header stays generic rather than repeating it.
            (Some("Resources".to_string()), c.as_slice(), Some(p.clone()))
        } else {
            (None, &[], None)
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
            // Images (Pict) and sounds (Snd) are clickable to preview (SQ-0347):
            // style them like a link and map their line to the resource so a
            // click can re-read the bytes and pop the modal.
            let kind = match c.usage.trim() {
                "Pict" => Some(PreviewKind::Image),
                "Snd" => Some(PreviewKind::Sound),
                _ => None,
            };
            match (kind, &blorb_path) {
                (Some(kind), Some(bp)) => {
                    resource_refs.push((
                        lines.len(),
                        ResourceRef {
                            blorb_path: bp.clone(),
                            kind,
                            number: c.number,
                            label: format!("{} #{}", resource_usage_label(&c.usage), c.number),
                        },
                    ));
                    lines.push((line, cs.story_info_link));
                }
                _ => lines.push((line, cs.story_info_value)),
            }
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
    for (vi, (text, style)) in lines[eff..end].iter().enumerate() {
        let li = eff + vi;
        let y = inner.y + vi as u16;
        if let Some((_, url)) = link_urls.iter().find(|(idx, _)| *idx == li) {
            // OSC 8 hyperlink (SQ-0367): the whole visible label is clickable and
            // opens the full URL, so a truncated URL still works. Degrades to
            // plain styled text on terminals without hyperlink support.
            let rect = Rect::new(text_area.x, y, text_area.width, 1);
            let link = hyperrat::Link::new(text.as_str(), url.as_str()).style(*style);
            ratatui::widgets::Widget::render(link, rect, buf);
            // hyperrat packs the whole OSC 8 escape sequence into the first
            // cell's symbol but leaves its diff option at None, so ratatui's diff
            // measures the escape bytes as display width (huge) and then skips
            // that many following cells when flushing — leaving a stale label
            // tail and a corrupted scrollbar on this row. Pin the cell to width 1
            // (ratatui's documented remedy for escape-symbol cells); hyperrat's
            // own Skip cells already carry the label's real column span.
            if let Some(first) = buf.cell_mut(ratatui::layout::Position::new(rect.x, rect.y)) {
                first.set_diff_option(ratatui::buffer::CellDiffOption::ForcedWidth(
                    std::num::NonZeroU16::new(1).unwrap(),
                ));
            }
            link_rects.push((rect, url.clone()));
            continue;
        }
        if let Some((_, rref)) = resource_refs.iter().find(|(idx, _)| *idx == li) {
            // A previewable Pict/Snd row (SQ-0347): draw it, and record its rect
            // so a click can open the resource preview modal.
            draw_str_clipped(buf, text_area.x, y, text, *style, text_area);
            resource_rects.push((Rect::new(text_area.x, y, text_area.width, 1), rref.clone()));
            continue;
        }
        draw_str_clipped(buf, text_area.x, y, text, *style, text_area);
    }
    if overflow {
        let sb_area = Rect::new(inner.right().saturating_sub(1), inner.y, 1, inner.height);
        app::render::scroll::draw_scrollbar(buf, sb_area, lines.len(), inner.height as usize, eff, cs.scrollbar);
    }
    max_scroll
}

/// Load a clicked resource into a preview (SQ-0347). For an image, decode its
/// `Pict` bytes; for a sound, play it once through `audio` (constructed lazily
/// on first use, then held by the caller). Always returns a preview — an
/// unreadable blorb, an undecodable image, or a silent audio backend surfaces
/// as a status line rather than nothing.
fn open_resource_preview(
    rref: &ResourceRef,
    audio: &mut Option<audio::AudioBackend>,
    volume: u8,
) -> ResourcePreview {
    let blorb = std::fs::read(&rref.blorb_path)
        .ok()
        .and_then(|bytes| blorb::Blorb::parse(bytes).ok());
    match rref.kind {
        PreviewKind::Image => {
            let image = blorb
                .as_ref()
                .and_then(|b| b.resource(b"Pict", rref.number))
                .and_then(|(_, data)| app::cover::decode(data));
            let status = image
                .is_none()
                .then(|| "Can't preview this image (unsupported format).".to_string());
            ResourcePreview { title: rref.label.clone(), image, proto: None, status }
        }
        PreviewKind::Sound => {
            let status = play_preview_sound(blorb.as_ref(), rref.number, audio, volume);
            ResourcePreview { title: rref.label.clone(), image: None, proto: None, status: Some(status) }
        }
    }
}

/// Play sound resource `number` from `blorb` once, returning the status line for
/// the modal. Lazily constructs the audio backend into `audio` on first use.
fn play_preview_sound(
    blorb: Option<&blorb::Blorb>,
    number: u32,
    audio: &mut Option<audio::AudioBackend>,
    volume: u8,
) -> String {
    let Some(blorb) = blorb else {
        return "Couldn't read the resource blorb.".to_string();
    };
    let Some((bytes, kind)) = blorb.sound(number) else {
        return "Sound resource not found.".to_string();
    };
    let Some(fmt) = app::state::sound_kind_to_format(kind) else {
        return "Unsupported sound format.".to_string();
    };
    let backend = audio.get_or_insert_with(|| audio::AudioBackend::new(volume));
    match backend.play_sample(bytes, fmt, 8, 1) {
        Some(_) => "Playing…   (Esc / Enter to close)".to_string(),
        None => "No audio output available.".to_string(),
    }
}

/// Draw the resource-preview modal (SQ-0347) centred over `area`: dialog chrome
/// (border, title, ✕ close, a Close button) with the fitted image — or a status
/// line — in its content rect. Returns the dialog's hit rects for dismissal.
fn draw_resource_preview(
    pv: &mut ResourcePreview,
    area: Rect,
    picker: Option<&ratatui_image::picker::Picker>,
    cs: &app::colors::ColorScheme,
    buf: &mut ratatui::buffer::Buffer,
) -> app::render::dialog::DialogRects {
    use app::render::dialog::{draw_dialog, ButtonId, DialogButton, DialogSpec, DialogStyle, Placement};
    // Centre a generous box: 80% of the terminal, floored so it stays usable on
    // small screens and never exceeds the area.
    let w = ((area.width as u32 * 4 / 5) as u16).clamp(20, area.width).max(1);
    let h = ((area.height as u32 * 4 / 5) as u16).clamp(6, area.height).max(1);
    let st = DialogStyle::from_colors(cs);
    let buttons = [DialogButton { id: ButtonId::Close, label: "Close" }];
    let spec = DialogSpec {
        title: &pv.title,
        placement: Placement::Centered { w, h },
        buttons: &buttons,
        show_close: true,
        default: Some(ButtonId::Close),
        focus: None,
        field: None,
    };
    let rects = draw_dialog(buf, area, &spec, &st);

    let content = rects.content;
    // Render the image fitted + centred, else the status line centred.
    let mut drew_image = false;
    if let (Some(picker), Some(img)) = (picker, pv.image.as_ref()) {
        if content.width >= 1 && content.height >= 1 {
            let fresh = matches!(&pv.proto, Some((w, h, _)) if *w == content.width && *h == content.height);
            if !fresh {
                if let Ok(built) = picker.new_protocol(
                    img.clone(),
                    ratatui::layout::Size::new(content.width, content.height),
                    ratatui_image::Resize::Fit(None),
                ) {
                    pv.proto = Some((content.width, content.height, built));
                }
            }
            if let Some((_, _, proto)) = &pv.proto {
                let sz = proto.size();
                let uw = sz.width.min(content.width);
                let uh = sz.height.min(content.height);
                let dest = Rect::new(
                    content.x + (content.width - uw) / 2,
                    content.y + (content.height - uh) / 2,
                    uw,
                    uh,
                );
                ratatui::widgets::Widget::render(ratatui_image::Image::new(proto), dest, buf);
                drew_image = true;
            }
        }
    }
    if !drew_image {
        if let Some(status) = &pv.status {
            let text = truncate_to_width(status, content.width as usize);
            let tx = content.x + (content.width.saturating_sub(UnicodeWidthStr::width(text.as_str()) as u16)) / 2;
            let ty = content.y + content.height / 2;
            draw_str_clipped(buf, tx, ty, &text, cs.story_info_value, content);
        }
    }
    rects
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

    #[test]
    fn interp_label_formats_type_version_and_blorb() {
        use app::picker::{Engine, Features, StoryMeta};
        let meta = |engine: Engine, version: Option<&str>| StoryMeta {
            size_bytes: 0, modified: None, engine, format: String::new(),
            version: version.map(String::from), serial: None, release: None, ifid: String::new(),
            features: Features::default(), self_blorb: None, author: None, year: None,
            genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
        };
        // Z-code: "Z<v>", plus " (blorb)" only when blorb'd.
        assert_eq!(super::interp_label(&meta(Engine::ZCode, Some("5")), false), "Z5");
        assert_eq!(super::interp_label(&meta(Engine::ZCode, Some("3")), true), "Z3 (blorb)");
        assert_eq!(super::interp_label(&meta(Engine::ZCode, None), false), "Z");
        // Glulx: "G<v>", never a blorb suffix (Glulx is effectively always blorbed).
        assert_eq!(super::interp_label(&meta(Engine::Glulx, Some("3.1.2")), true), "G3.1.2");
        assert_eq!(super::interp_label(&meta(Engine::Glulx, None), false), "Glulx");
        // Widest label fits the column.
        assert!(super::interp_label(&meta(Engine::ZCode, Some("8")), true).len() <= super::INTERP_COL_W as usize);
    }

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
                author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
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
                genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
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
        // Type AND blorb moved into the TYPE column (SQ-0369), so the badge
        // cluster is just [save][hint], adjacent and no separators.
        assert!(row0.contains("SH"), "save+hint adjacent, no type/blorb glyph: {row0:?}");
        assert!(row1.contains("S"), "got: {row1:?}");
        assert!(!row1.contains("H"), "absent hint omitted: {row1:?}");
        // The blorb'd Z-code story shows "(blorb)" in its TYPE column; the Glulx
        // story shows its interpreter label. Neither shows a B badge.
        assert!(row0.contains("(blorb)"), "blorb'd Z story tagged in TYPE column: {row0:?}");
        assert!(row1.contains("Glulx"), "Glulx story shows its type label: {row1:?}");

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
        sym.badge_zcode = "z!".into();  // moved to the TYPE column (SQ-0369)
        sym.badge_blorb = "◆".into();   // ditto, as " (blorb)"
        sym.badge_save = "§".into();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);

        let stories = make_two_test_stories();
        let badges = vec![
            app::picker::RowBadges { blorb: true, save: true, hint: false },
            app::picker::RowBadges::default(),
        ];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(stories.len());
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(&stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
                          &cs, app::picker::Sort::default(), area, &mut buf);
        let row0 = row_text(&buf, 2, area);
        // The configured save glyph is used for the artifact badge. Type and
        // blorb are no longer badges (they're the TYPE column), so their
        // configured glyphs must NOT appear as badges in the row.
        assert!(row0.contains('§'), "configured save glyph used: {row0:?}");
        assert!(!row0.contains("z!"), "type is the TYPE column now, not a badge: {row0:?}");
        assert!(!row0.contains('◆'), "blorb is the TYPE column now, not a badge: {row0:?}");
        assert!(row0.contains("(blorb)"), "blorb shown as a TYPE suffix instead: {row0:?}");
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

        // (width, author shown, year shown). Right zone = INTERP_COL_W(10) +
        // COL_GAP(2) + cluster_w(save+hint=2) = 14, reserved 15; so avail =
        // width - 17. year needs avail >= 38 (width >= 55); author needs avail
        // >= 30 (width >= 47). Below that: title + right-zone only.
        for &(width, want_author, want_year) in &[
            (70u16, true, true),
            (55, true, true),
            (54, true, false),
            (47, true, false),
            (46, false, false),
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

            // The TYPE column stays right-aligned at a fixed offset regardless of
            // which text columns show — proving no gap opened in front of the
            // right-hand zone as columns drop. This story is ZCode/no-version/
            // no-blorb, so its interpreter label is "Z".
            let interp_x = width - 1 - 2 /*cluster*/ - super::COL_GAP - super::INTERP_COL_W;
            let cell = buf.cell((interp_x, 2)).unwrap();
            assert_eq!(cell.symbol(), "Z", "TYPE column at col {interp_x} for width {width}: {row:?}");
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
        // TYPE column ("Z" here) stays put at its fixed right-zone offset.
        let interp_x = 60u16 - 1 - 2 - super::COL_GAP - super::INTERP_COL_W;
        assert_eq!(
            buf.cell((interp_x, 2)).unwrap().symbol(), "Z",
            "TYPE column unaffected by the author overrun"
        );
    }

    #[test]
    fn author_column_grows_to_show_a_longer_name_when_there_is_room() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let sym = app::config::SymbolConfig::default();
        let glyphs = app::picker::BadgeGlyphs::from_symbols(&sym);
        // 27 wide — past the old fixed 20-col author column, but under the cap.
        let author = "Brian Moriarty (Infocom)  X";
        let stories = vec![story_with_meta("Trinity", Some(author), Some("1986"))];
        let badges = vec![app::picker::RowBadges::default()];
        let mut list = app::list_scroll::ListScroll::new();
        list.len(1);
        // A wide terminal: title's minimum is easily met, so the author column
        // should grow to show the whole name rather than truncate at 20.
        let area = Rect::new(0, 0, 100, 10);
        let mut buf = Buffer::empty(area);
        super::draw_story_picker(
            &stories, &list, &badges, &glyphs, std::path::Path::new("/tmp"),
            &cs, app::picker::Sort::default(), area, &mut buf,
        );
        let row = row_text(&buf, 2, area);
        assert!(row.contains(author), "author shown in full when there is room: {row:?}");
        assert!(!row.contains('…'), "no ellipsis when the column grew to fit: {row:?}");
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

        // Segments appear in FOOTER_OPTIONAL order as width grows (least-
        // guessable first): each one's minimum fitting width is >= the previous
        // one's, and the core is always present. Robust to segment-set changes.
        let min_width = |seg: &str| -> u16 {
            (10u16..=240).find(|&w| super::build_footer(w).contains(seg)).unwrap_or(u16::MAX)
        };
        let widths: Vec<u16> = super::FOOTER_OPTIONAL.iter().map(|s| min_width(s)).collect();
        for pair in widths.windows(2) {
            assert!(pair[0] <= pair[1], "segments appear in declared order: {widths:?}");
        }
        // f: fetch (first, least guessable) appears well before PgUp/PgDn (last).
        assert!(widths[0] < *widths.last().unwrap(), "{widths:?}");
        // At a wide-enough terminal, every optional hint (incl. the new u) shows.
        let wide = super::build_footer(200);
        for seg in super::FOOTER_OPTIONAL {
            assert!(wide.contains(seg), "wide footer shows {seg:?}: {wide:?}");
        }
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
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
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
            "Zork I", "zork1.z3", &meta, Some(&aux), 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
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
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 34, 10);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("zork1.z3");
        let max_scroll = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text_top = buffer_to_string(&buf, area);
        assert!(max_scroll > 0, "content should overflow a 10-row panel");
        let late_marker = " #29  ";
        assert!(!text_top.contains(late_marker), "late resource should be offscreen at scroll 0: {text_top:?}");

        let mut buf2 = Buffer::empty(area);
        let max_scroll2 = super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, max_scroll, area, None, &mut cover, entry_path, false, &cs, &mut buf2, &mut Vec::new(), &mut Vec::new(),
        );
        let text_scrolled = buffer_to_string(&buf2, area);
        assert_eq!(max_scroll2, max_scroll);
        assert!(text_scrolled.contains(late_marker), "late resource should be visible when scrolled: {text_scrolled:?}");

        // Scrolling past max clamps to the same view as scroll == max_scroll.
        let mut buf3 = Buffer::empty(area);
        super::draw_info_panel(
            "Zork I", "zork1.z3", &meta, None, 999, area, None, &mut cover, entry_path, false, &cs, &mut buf3, &mut Vec::new(), &mut Vec::new(),
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
            author: None, year: None, genre: None, language: None, description: None, ifdb_link: None, fetch_not_found: false,
        }
    }

    // ── SQ-0348: author/year/genre + blurb ──────────────────────────────────────

    /// A story with NO fetched/embedded metadata must render exactly as it did
    /// before this feature existed: no empty "Author:" label, no stray blank
    /// line, no separator with nothing either side of it. The IFID line and
    /// the Features line (present since `minimal_story_meta` has no features by
    /// default, so give it one) must land on directly adjacent rows.
    #[test]
    fn info_panel_no_metadata_leaves_ifid_and_features_adjacent() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.features = app::picker::Features { sound: true, ..Default::default() };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        let lines: Vec<&str> = text.lines().collect();
        // Row 0 is the panel's own top border (with the " Info " title baked
        // into it); content starts at row 1: title, filename/size,
        // format/version, IFID, Features (no serial, no metadata).
        assert!(lines[4].contains("IFID"), "row 4 should be the IFID line: {:?}", lines[4]);
        assert!(
            lines[5].trim_start_matches('│').trim().starts_with("Features:"),
            "no line should be inserted between IFID and Features when metadata is absent: {:?}",
            lines[5]
        );
        assert!(!text.contains("Author"), "no metadata label should appear: {text:?}");
    }

    /// With author/year/genre and a blurb present, a combined "author · year ·
    /// genre" line and the wrapped blurb text land between IFID and Features,
    /// disturbing neither.
    #[test]
    fn info_panel_renders_author_year_genre_and_blurb_between_ifid_and_features() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.author = Some("Michael S. Gentry".into());
        meta.year = Some("1998".into());
        meta.genre = Some("Horror".into());
        meta.description = Some("A tale of terror in a small town.".into());
        meta.features = app::picker::Features { sound: true, ..Default::default() };
        let area = Rect::new(0, 0, 50, 14);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Michael S. Gentry"), "author should render: {text:?}");
        assert!(text.contains("1998"), "year should render: {text:?}");
        assert!(text.contains("Horror"), "genre should render: {text:?}");
        assert!(text.contains("A tale of terror in a small town."), "blurb should render: {text:?}");

        let ifid_pos = text.find("IFID").expect("IFID line present");
        let author_pos = text.find("Michael S. Gentry").expect("author present");
        let blurb_pos = text.find("A tale of terror").expect("blurb present");
        let features_pos = text.find("Features:").expect("features present");
        assert!(ifid_pos < author_pos, "author line must come after IFID");
        assert!(author_pos < blurb_pos, "blurb must come after the author/year/genre line");
        assert!(blurb_pos < features_pos, "blurb must come before Features");
    }

    /// SQ-0372: a story stored beside a separate resource `.blorb` names that
    /// sidecar file up-front in the metadata block (not only in the Resources
    /// header), so it is visible without scrolling.
    #[test]
    fn info_panel_names_the_sidecar_resource_blorb() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let meta = minimal_story_meta(); // self_blorb: None → the sidecar path
        let aux = app::picker::StoryAux {
            assoc_blorb: Some((
                std::path::PathBuf::from("/games/beyondzork.blb"),
                vec![app::picker::ChunkInfo {
                    usage: "Pict".into(), number: 1, chunk_type: "PNG ".into(), len: 4096, detail: None,
                }],
            )),
            saves: vec![],
            hints_available: false,
            game_dir: std::path::PathBuf::from("/tmp/bz"),
            qzl_saves: vec![],
            auto_saves: vec![],
            sidecars: vec![],
        };
        let area = Rect::new(0, 0, 60, 16);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        super::draw_info_panel(
            "Beyond Zork", "beyondzork-r57-s871221.z5", &meta, Some(&aux), 0, area, None,
            &mut cover, std::path::Path::new("beyondzork-r57-s871221.z5"), false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        let text = buffer_to_string(&buf, area);
        assert!(text.contains("Resource blorb: beyondzork.blb"), "sidecar named up-front: {text:?}");
        assert!(text.contains("Image"), "the sidecar's Pict resource still lists: {text:?}");
        let blorb_pos = text.find("Resource blorb:").expect("sidecar line present");
        let res_pos = text.find("Resources").expect("resources header present");
        assert!(blorb_pos < res_pos, "sidecar name is up-front, before the Resources section");
    }

    /// Regression (SQ-0367 link vs scrollbar): hyperrat leaves the OSC 8 first
    /// cell's diff option at None, so ratatui measures the escape sequence as the
    /// cell's width and skips the rest of the row — a stale label tail and a
    /// corrupted scrollbar. We must pin that cell to ForcedWidth(1).
    #[test]
    fn info_panel_link_cell_is_pinned_to_width_one() {
        use ratatui::buffer::{Buffer, CellDiffOption};
        use ratatui::layout::{Position, Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.ifdb_link = Some("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".into());
        let area = Rect::new(0, 0, 60, 14);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut links: Vec<(Rect, String)> = Vec::new();
        super::draw_info_panel(
            "Game", "game.z5", &meta, None, 0, area, None, &mut cover,
            std::path::Path::new("game.z5"), false, &cs, &mut buf, &mut links, &mut Vec::new(),
        );
        let (rect, _) = links.first().expect("a link rect was recorded");
        let first = buf.cell(Position::new(rect.x, rect.y)).expect("link first cell");
        assert!(
            matches!(first.diff_option, CellDiffOption::ForcedWidth(w) if w.get() == 1),
            "link first cell must be pinned to width 1, was {:?}", first.diff_option
        );
    }

    #[test]
    fn info_panel_shows_the_ifdb_link_only_once_fetched() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let render = |meta: &app::picker::StoryMeta| {
            let mut buf = Buffer::empty(area);
            let mut cover = app::cover::CoverState::default();
            super::draw_info_panel(
                "Game", "game.z5", meta, None, 0, area, None, &mut cover,
                std::path::Path::new("game.z5"), false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
            );
            buffer_to_string(&buf, area)
        };
        // Not fetched → no IFDB line at all.
        let bare = minimal_story_meta();
        assert!(!render(&bare).contains("IFDB:"), "no link before a fetch");
        // Fetched → the bare URL renders (terminals auto-link it).
        let mut fetched = minimal_story_meta();
        fetched.ifdb_link = Some("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".into());
        assert!(
            render(&fetched).contains("https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro"),
            "the IFDB URL renders once present"
        );
    }

    /// SQ-0371: a fetch that found nothing offers a manual IFDB search link (by
    /// title) instead of a dead end — but a never-fetched story shows neither.
    #[test]
    fn info_panel_offers_a_search_link_when_the_fetch_found_nothing() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let render = |title: &str, meta: &app::picker::StoryMeta| {
            let mut buf = Buffer::empty(area);
            let mut cover = app::cover::CoverState::default();
            super::draw_info_panel(
                title, "game.z5", meta, None, 0, area, None, &mut cover,
                std::path::Path::new("game.z5"), false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
            );
            buffer_to_string(&buf, area)
        };
        // Never fetched → no search link (only `f`/`r` offers a fetch).
        let bare = minimal_story_meta();
        assert!(!render("Zork I", &bare).contains("IFDB search:"), "no search link before a fetch");
        // Fetch ran, found nothing → a search-by-title link appears.
        let mut nf = minimal_story_meta();
        nf.fetch_not_found = true;
        let out = render("Zork I", &nf);
        assert!(out.contains("IFDB search:"), "not-found offers a manual search: {out:?}");
        assert!(out.contains("ifdb.org/search?searchfor=Zork"), "search is by title: {out:?}");
        // A successful link takes precedence over the search fallback.
        let mut found = minimal_story_meta();
        found.fetch_not_found = true; // even if the flag is stale
        found.ifdb_link = Some("https://ifdb.org/viewgame?id=abc".into());
        let out = render("Zork I", &found);
        assert!(out.contains("viewgame?id=abc") && !out.contains("IFDB search:"), "link wins: {out:?}");
    }

    /// SQ-0367: the info panel reports the screen rect + URL of each OSC 8 link
    /// it draws, so the picker loop can open one on a click (mouse capture keeps
    /// the terminal from acting on the hyperlink itself). No link → empty.
    #[test]
    fn info_panel_surfaces_the_link_rect_for_click_to_open() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 60, 14);
        let mut cover = app::cover::CoverState::default();
        let path = std::path::Path::new("game.z5");

        // No fetch → no link → no rects.
        let mut buf = Buffer::empty(area);
        let mut rects: Vec<(Rect, String)> = Vec::new();
        super::draw_info_panel(
            "Zork I", "game.z5", &minimal_story_meta(), None, 0, area, None, &mut cover,
            path, false, &cs, &mut buf, &mut rects, &mut Vec::new(),
        );
        assert!(rects.is_empty(), "no link rects before a fetch: {rects:?}");

        // Fetched → one link rect carrying the full URL, inside the panel.
        let mut fetched = minimal_story_meta();
        let url = "https://ifdb.org/viewgame?id=0dbnusxunq7fw5ro".to_string();
        fetched.ifdb_link = Some(url.clone());
        let mut buf = Buffer::empty(area);
        rects.clear();
        super::draw_info_panel(
            "Zork I", "game.z5", &fetched, None, 0, area, None, &mut cover,
            path, false, &cs, &mut buf, &mut rects, &mut Vec::new(),
        );
        assert_eq!(rects.len(), 1, "one link rect once fetched: {rects:?}");
        let (rect, got) = &rects[0];
        assert_eq!(got, &url, "rect carries the full URL for opening");
        assert!(area.contains(rect.as_position()), "link rect is inside the panel");
    }

    /// A long blurb wraps to the panel's content width and, when it overflows
    /// the panel height, scrolls with the SAME `panel_scroll`/`panel_max`
    /// mechanism as the rest of the info panel (no second scroll system).
    #[test]
    fn info_panel_blurb_wraps_and_participates_in_panel_scroll() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        meta.description = Some(
            "one two three four five six seven eight nine ten eleven twelve \
             thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty"
                .into(),
        );
        // Narrow + short so the wrapped blurb both wraps to multiple lines and
        // overflows the panel height.
        let area = Rect::new(0, 0, 20, 8);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        let max_scroll = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        assert!(max_scroll > 0, "a long wrapped blurb should overflow an 8-row panel");
        let text_top = buffer_to_string(&buf, area);
        assert!(!text_top.contains("twenty"), "late blurb word should be offscreen at scroll 0: {text_top:?}");

        let mut buf2 = Buffer::empty(area);
        let max_scroll2 = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, max_scroll, area, None, &mut cover, entry_path, false, &cs, &mut buf2, &mut Vec::new(), &mut Vec::new(),
        );
        assert_eq!(max_scroll2, max_scroll, "max_scroll must be stable across scroll positions");
        let text_scrolled = buffer_to_string(&buf2, area);
        assert!(text_scrolled.contains("twenty"), "late blurb word should be visible once scrolled: {text_scrolled:?}");
    }

    /// Regression (word-wrap vs scrollbar): when the blurb overflows and the
    /// scrollbar claims the last column, the wrap must reserve that gutter so a
    /// word landing on the panel's right edge is not clipped by one character.
    #[test]
    fn info_panel_blurb_wrap_reserves_the_scrollbar_gutter() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let mut meta = minimal_story_meta();
        // In the 18-col inner width, "aaaaaaaa bbbbbbbbb" is exactly 18 wide: it
        // fills the full inner width but not the width-1 text area left once the
        // scrollbar takes a column. Wrapping to the full width clips the last
        // 'b'; reserving the gutter pushes "bbbbbbbbb" to its own line intact.
        meta.description = Some(
            "aaaaaaaa bbbbbbbbb cccccccc dddddddd eeeeeeee ffffffff gggggggg hhhhhhhh iiiiiiii jjjjjjjj"
                .into(),
        );
        let area = Rect::new(0, 0, 20, 10); // inner 18x8 → 10 content lines overflow
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("game.gblorb");
        let max_scroll = super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
        );
        assert!(max_scroll > 0, "blurb should overflow so the scrollbar shows");
        let text = buffer_to_string(&buf, area);
        assert!(
            text.contains("bbbbbbbbb"),
            "the 9-char word must not be clipped by the scrollbar column: {text:?}"
        );
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
            0, area, Some(&picker), &mut cover, &path, false, &cs, &mut buf, &mut Vec::new(), &mut Vec::new(),
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

    // ── SQ-0348: fetch-progress wiring ──────────────────────────────────────────
    //
    // `run_story_picker` itself can't be unit-tested (it owns a real terminal),
    // so these exercise the pieces the loop wires together: `resort_list`
    // (the caches stay index-aligned with `stories`), the progress-line
    // overlay, and — the important one — a simulated `Fetcher` sweep driving
    // `resolve_entry` + `resort_preserving_selection` exactly as the loop's
    // drain handler does, proving the selection survives titles landing mid-sweep.

    /// Minimal valid v3 story bytes (mirrors `picker.rs`'s private test fixture
    /// of the same name — not reusable across modules, so duplicated here).
    fn minimal_v3_story() -> Vec<u8> {
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        buf[0x18] = 0x00; buf[0x19] = 0x60;
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0;
        buf
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!("babelmap-picker-ui-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resort_list_keeps_row_badges_and_aux_cache_aligned_with_the_new_order() {
        let stories_dir = temp_dir("resort-align");
        std::fs::write(stories_dir.join("a.z5"), minimal_v3_story()).unwrap();
        let mut b_bytes = minimal_v3_story();
        b_bytes[0x12] = b'9'; // distinct serial → distinct IFID from a.z5
        std::fs::write(stories_dir.join("b.z5"), b_bytes).unwrap();
        let data_base = temp_dir("resort-align-data");
        let hint_index = app::hints::load_hint_index(&data_base);

        let mut stories = app::picker::scan_stories(&stories_dir, &data_base);
        assert_eq!(stories.len(), 2);
        let mut row_badges: Vec<app::picker::RowBadges> = stories
            .iter()
            .map(|e| app::picker::compute_row_badges(e, &data_base, &hint_index))
            .collect();
        let mut aux_cache: Vec<Option<app::picker::StoryAux>> = vec![Some(app::picker::resolve_aux(
            &stories[0],
            &data_base,
            &hint_index,
        ))];
        aux_cache.push(None);
        let selected_path = stories[0].path.clone();

        let new_idx = super::resort_list(
            &mut stories,
            0,
            app::picker::Sort { key: app::picker::SortKey::Title, desc: true },
            &mut row_badges,
            &mut aux_cache,
            &data_base,
            &hint_index,
        );

        assert_eq!(stories[new_idx].path, selected_path, "selection follows its story");
        assert_eq!(row_badges.len(), stories.len(), "row_badges stays index-aligned");
        assert_eq!(aux_cache.len(), stories.len(), "aux_cache stays index-aligned");
        assert!(aux_cache.iter().all(Option::is_none), "a reorder invalidates every cached aux slot");

        let _ = std::fs::remove_dir_all(&stories_dir);
        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn progress_line_overlays_the_footer_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        // Pre-fill the footer row with something the overlay must fully replace,
        // proving it clears trailing characters rather than just prefixing.
        for x in area.left()..area.right() {
            if let Some(c) = buf.cell_mut((x, area.bottom() - 1)) {
                c.set_symbol("#");
            }
        }
        super::draw_progress_line(&mut buf, area, "Fetching 7/23 — Zork I", cs.story_header_active);
        let row = row_text(&buf, area.bottom() - 1, area);
        assert!(row.contains("Fetching 7/23 — Zork I"), "{row:?}");
        assert!(!row.contains('#'), "the overlay must clear the whole row, not just prefix it: {row:?}");
        let cell = buf.cell((area.left(), area.bottom() - 1)).unwrap();
        assert_eq!(
            Some(cell.fg), cs.story_header_active.fg,
            "progress line must use a themed style, not a hard-coded color"
        );
    }

    /// A `MetadataSource` fake local to this module (the one in
    /// `fetch_worker`'s tests is private to that module) — canned responses
    /// keyed by IFID, never touching the network.
    struct FakeSource {
        title_by_ifid: std::collections::HashMap<String, String>,
    }

    impl app::ifdb::MetadataSource for FakeSource {
        fn fetch(&self, ifid: &str) -> Result<app::ifdb::FetchOutcome, app::ifdb::FetchError> {
            match self.title_by_ifid.get(ifid) {
                Some(title) => Ok(app::ifdb::FetchOutcome::Found(Box::new(app::ifiction::IFiction {
                    title: Some(title.clone()),
                    ..Default::default()
                }))),
                None => Ok(app::ifdb::FetchOutcome::NotFound),
            }
        }
        fn fetch_by_id(&self, _tuid: &str) -> Result<app::ifdb::FetchOutcome, app::ifdb::FetchError> {
            Ok(app::ifdb::FetchOutcome::NotFound)
        }
        fn fetch_cover(&self, _url: &str) -> Result<Vec<u8>, app::ifdb::FetchError> {
            Ok(Vec::new())
        }
    }

    /// THE highest-value integration test in this task: drives a real
    /// `Fetcher` (with a fake source, zero delay) over two stories, then runs
    /// the exact same drain-handling pipeline the picker loop uses —
    /// `resolve_entry` to pick up the freshly-written sidecar, then
    /// `resort_preserving_selection` — and checks the selection followed its
    /// story through a title-driven reorder, not its index.
    #[test]
    fn a_simulated_sweep_lands_new_titles_and_the_selection_follows_its_story() {
        let stories_dir = temp_dir("sweep");
        // "zork2.z5" starts as a bare stem title that sorts LAST (after the
        // untouched "other.z5" control story); the sweep gives it a fetched
        // title that sorts FIRST, so a naive index-based cursor would end up
        // pointing at the wrong (unrelated) story once the sweep lands.
        std::fs::write(stories_dir.join("other.z5"), minimal_v3_story()).unwrap();
        let mut b_bytes = minimal_v3_story();
        b_bytes[0x12] = b'9';
        std::fs::write(stories_dir.join("zork2.z5"), b_bytes.clone()).unwrap();
        let data_base = temp_dir("sweep-data");

        let mut stories = app::picker::scan_stories(&stories_dir, &data_base);
        assert_eq!(stories.len(), 2);
        let selected = stories.iter().position(|e| e.path.ends_with("zork2.z5")).unwrap();
        let ifid_b = stories[selected].meta.ifid.clone();

        let mut title_by_ifid = std::collections::HashMap::new();
        title_by_ifid.insert(ifid_b, "AAA Brand New Title".to_string());
        let fetcher = app::fetch_worker::Fetcher::new(
            Box::new(FakeSource { title_by_ifid }),
            data_base.clone(),
            std::time::Duration::ZERO,
        );
        let order: Vec<(std::path::PathBuf, String)> =
            stories.iter().map(|e| (e.path.clone(), e.meta.ifid.clone())).collect();
        fetcher.request(app::fetch_worker::FetchOrder { stories: order, forced: true, id_override: None });

        // Bounded drain (mirrors fetch_worker's own test pattern): collect
        // progress until both stories report in, or give up after ~2s.
        let mut progress = Vec::new();
        for _ in 0..2000 {
            progress.extend(fetcher.drain());
            if progress.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(progress.len(), 2, "both stories must report a completed fetch");

        // Exactly what the picker loop's drain handler does per progress item.
        for p in &progress {
            if let Some(fresh) = app::picker::resolve_entry(&p.path, &data_base) {
                if let Some(slot) = stories.iter_mut().find(|e| e.path == p.path) {
                    *slot = fresh;
                }
            }
        }
        assert_eq!(stories[selected].title, "AAA Brand New Title", "the sidecar write landed");

        let new_idx =
            app::picker::resort_preserving_selection(&mut stories, selected, app::picker::Sort::default());
        assert_eq!(new_idx, 0, "the new title now sorts first");
        assert!(stories[new_idx].path.ends_with("zork2.z5"), "selection followed its story, not its old index");
        assert_eq!(stories[new_idx].title, "AAA Brand New Title");

        let _ = std::fs::remove_dir_all(&stories_dir);
        let _ = std::fs::remove_dir_all(&data_base);
    }

    #[test]
    fn gallery_centers_titles_for_missing_covers_and_glows_selection() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        let stories = vec![
            story_with_meta("Zork", None, None),
            story_with_meta("Anchorhead", None, None),
            story_with_meta("Curses", None, None),
        ];
        let area = Rect::new(0, 0, 80, 40);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut first_row = 0usize;
        // No picker → no cover art → each tile shows its title centred in the band.
        let (rects, cols, vis) = super::draw_story_gallery(
            &stories, 1, &mut first_row, std::path::Path::new("/tmp"), &cs, None, &mut cover, area, &mut buf,
        );

        assert!(cols >= 1 && vis >= 1);
        assert_eq!(rects.len(), 3, "one hit-rect per story tile");
        // Missing covers render the title centred in the cover band itself.
        let whole: String = (area.top()..area.bottom())
            .map(|y| row_text(&buf, y, area))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(whole.contains("Zork"), "missing-cover title: {whole:?}");
        assert!(whole.contains("Anchorhead"));
        assert!(whole.contains("Curses"));

        // The selected tile (index 1) is tinted with the selection style (the
        // glow); an unselected tile is not. The tile is the cover band alone now,
        // so any cell in it carries the tile's fill style.
        let sel_tile = rects.iter().find(|(i, _)| *i == 1).unwrap().1;
        let sel_cell = buf.cell((sel_tile.x, sel_tile.y)).unwrap();
        assert_eq!(sel_cell.style().bg, cs.story_tile_selected.bg, "selected tile glows");
        let unsel_tile = rects.iter().find(|(i, _)| *i == 0).unwrap().1;
        let unsel_cell = buf.cell((unsel_tile.x, unsel_tile.y)).unwrap();
        assert_ne!(unsel_cell.style().bg, cs.story_tile_selected.bg, "unselected tile is not tinted");
    }

    /// A 2×2 red PNG, encoded via the `image` crate (mirrors cover.rs fixtures).
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 0, 0]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// A minimal blorb carrying one `Pict` resource (number 1) holding `png`.
    fn blorb_with_pict(png: &[u8]) -> Vec<u8> {
        fn iff_chunk(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut v = Vec::new();
            v.extend_from_slice(ty);
            v.extend_from_slice(&(data.len() as u32).to_be_bytes());
            v.extend_from_slice(data);
            if data.len() % 2 == 1 {
                v.push(0);
            }
            v
        }
        let mut ridx = Vec::new();
        ridx.extend_from_slice(&1u32.to_be_bytes()); // one entry
        ridx.extend_from_slice(b"Pict");
        ridx.extend_from_slice(&1u32.to_be_bytes()); // number
        let ridx_chunk_len = 8 + (4 + 12); // header + count + one 12-byte entry
        let pict_off = 12 + ridx_chunk_len; // FORM/IFRS header + RIdx chunk
        ridx.extend_from_slice(&(pict_off as u32).to_be_bytes());
        let mut inner = Vec::new();
        inner.extend_from_slice(b"IFRS");
        inner.extend_from_slice(&iff_chunk(b"RIdx", &ridx));
        inner.extend_from_slice(&iff_chunk(b"PNG ", png));
        let mut file = Vec::new();
        file.extend_from_slice(b"FORM");
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    #[test]
    fn info_panel_records_a_hit_rect_for_a_previewable_pict_row() {
        use ratatui::{buffer::Buffer, layout::Rect};
        use app::picker::{ChunkInfo, Engine, Features, StoryMeta};
        let cs = app::colors::ColorScheme::terminal_default();
        // A self-contained blorb story with one image resource.
        let meta = StoryMeta {
            size_bytes: 1, modified: None, engine: Engine::ZCode, format: "Blorb".into(),
            version: None, serial: None, release: None, ifid: "X".into(),
            features: Features::default(),
            self_blorb: Some(vec![ChunkInfo {
                usage: "Pict".into(), number: 3, chunk_type: "PNG ".into(), len: 100, detail: None,
            }]),
            author: None, year: None, genre: None, language: None, description: None,
            ifdb_link: None, fetch_not_found: false,
        };
        let area = Rect::new(0, 0, 40, 30);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let entry_path = std::path::Path::new("/tmp/game.gblorb");
        let mut resource_rects: Vec<(Rect, super::ResourceRef)> = Vec::new();
        super::draw_info_panel(
            "Game", "game.gblorb", &meta, None, 0, area, None, &mut cover, entry_path,
            false, &cs, &mut buf, &mut Vec::new(), &mut resource_rects,
        );
        assert_eq!(resource_rects.len(), 1, "the Pict row is clickable");
        let (_, rref) = &resource_rects[0];
        assert_eq!(rref.kind, super::PreviewKind::Image);
        assert_eq!(rref.number, 3);
        assert_eq!(rref.blorb_path, entry_path, "self-blorb resource reads from the story itself");
    }

    #[test]
    fn open_preview_decodes_an_image_resource_from_a_blorb() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("babelmap-preview-{}.blb", std::process::id()));
        std::fs::write(&path, blorb_with_pict(&tiny_png())).unwrap();

        let rref = super::ResourceRef {
            blorb_path: path.clone(),
            kind: super::PreviewKind::Image,
            number: 1,
            label: "Image #1".into(),
        };
        let mut audio = None;
        let pv = super::open_resource_preview(&rref, &mut audio, 100);
        let _ = std::fs::remove_file(&path);
        assert!(pv.image.is_some(), "the Pict PNG decoded");
        assert!(pv.status.is_none(), "a decodable image needs no status line");
        assert!(audio.is_none(), "an image preview never touches the audio backend");
    }

    #[test]
    fn open_preview_of_a_missing_blorb_yields_a_status_not_a_panic() {
        let rref = super::ResourceRef {
            blorb_path: std::path::PathBuf::from("/no/such/file.blb"),
            kind: super::PreviewKind::Image,
            number: 1,
            label: "Image #1".into(),
        };
        let mut audio = None;
        let pv = super::open_resource_preview(&rref, &mut audio, 100);
        assert!(pv.image.is_none());
        assert!(pv.status.is_some(), "an unreadable blorb surfaces a status line");
    }

    #[test]
    fn gallery_scroll_follows_selection_offscreen() {
        use ratatui::{buffer::Buffer, layout::Rect};
        let cs = app::colors::ColorScheme::terminal_default();
        // Enough stories that a low selection is on a grid row below the fold.
        let stories: Vec<_> = (0..40).map(|i| story_with_meta(&format!("S{i}"), None, None)).collect();
        // Small area → few visible rows, so a late index forces a scroll.
        let area = Rect::new(0, 0, 40, 22);
        let mut buf = Buffer::empty(area);
        let mut cover = app::cover::CoverState::default();
        let mut first_row = 0usize;
        let (rects, _cols, _vis) = super::draw_story_gallery(
            &stories, 39, &mut first_row, std::path::Path::new("/tmp"), &cs, None, &mut cover, area, &mut buf,
        );
        assert!(first_row > 0, "grid scrolled down to keep the last cover visible");
        assert!(rects.iter().any(|(i, _)| *i == 39), "the selected tile is on screen");
    }
}
