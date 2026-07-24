//! The story-picker's IFDB search modal (SQ-0413): a three-view state machine
//! (type a query → browse results → optionally pick among a game's download
//! files) plus its renderer. All network work is delegated to
//! [`crate::ifdb_search::SearchWorker`]; this module is the pure UI half — the
//! picker drives it by feeding keys/events in and dispatching the [`ModalAction`]s
//! it hands back. Every state transition is unit-tested without a network.
//!
//! Standing modal conventions (matching the picker's other modals): Enter
//! activates, Up/Down (and j/k) navigate lists. While a request is in flight
//! the modal is "busy": keystrokes are ignored except Esc, which abandons the
//! pending result.
//!
//! SQ-0473: the modal opens showing a "Popular on IFDB" seed list instead of
//! an empty query box — see [`SearchModal::open`] and the module's Esc-ladder
//! note below.
//!
//! Esc ladder (results → query → close), with SQ-0473's one twist: Esc from
//! the seed list closes the modal outright (one level shorter than before,
//! since there's no "empty query" screen behind it worth a stop); Esc from a
//! *typed* search's results returns to the seed list, cached from modal-open,
//! rather than to an empty box; Esc while typing (query box active) still
//! closes, unchanged. See `results_key` below.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::colors::ColorScheme;
use crate::ifdb_search::{DownloadOption, SearchEvent, SearchHit};
use crate::ifiction::IFiction;
use crate::render::dialog::{
    draw_dialog, DialogField, DialogRects, DialogSpec, DialogStyle, Placement,
};
use crate::text_field::TextField;

/// Which view the modal is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    /// Typing a query.
    Input,
    /// Browsing search hits.
    Results,
    /// Choosing among several playable download files for one game.
    Choosing,
}

/// The kind of request currently in flight (if any). Guards against a stale
/// result being applied after the user backed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Inflight {
    /// The "Popular on IFDB" seed query (SQ-0473), fetched once on open.
    Seed,
    Search,
    Resolve,
    Download,
}

/// What the picker should do in response to a key/event. The picker owns the
/// worker and the download directory, so it translates these into jobs.
#[derive(Debug, Clone, PartialEq)]
pub enum ModalAction {
    /// Nothing to do.
    None,
    /// Close the modal.
    Close,
    /// Dispatch a game search for this query.
    Search(String),
    /// Resolve download options for this IFDB tuid.
    Resolve(String),
    /// Download this file URL (the picker supplies the destination dir).
    Download(String),
    /// Open this IFDB page URL in the browser (the "no playable file" fallback).
    OpenInBrowser(String),
    /// Dispatch the "Popular on IFDB" seed query (SQ-0473) — returned once by
    /// [`SearchModal::open`], right after construction.
    Seed,
}

/// The modal's full state.
pub struct SearchModal {
    query: TextField,
    view: View,
    inflight: Option<Inflight>,
    hits: Vec<SearchHit>,
    hit_sel: usize,
    /// The "Popular on IFDB" seed list (SQ-0473), cached for this modal
    /// instance's lifetime once it arrives — reused when Esc backs out of a
    /// typed search rather than re-fetching. NOT persisted across separate
    /// modal opens (each `/` press builds a fresh `SearchModal`, so a second
    /// open re-fetches); see the module header.
    seed_hits: Vec<SearchHit>,
    /// True while `hits` holds the seed list rather than a typed search's
    /// results — governs the Esc ladder and the "Popular on IFDB" hint label.
    showing_seed: bool,
    options: Vec<DownloadOption>,
    opt_sel: usize,
    /// The iFiction record resolved alongside `options` (SQ-0474), if any —
    /// carried from the last `SearchEvent::Options` through to whichever
    /// `Download` action follows (immediately, for the one-option
    /// auto-download; later, for a user's pick in `View::Choosing`), so the
    /// picker can populate the download's sidecar + cover with zero extra
    /// requests. Taken (not cloned) by [`Self::take_pending_record`] once
    /// the picker dispatches that download. Boxed to match
    /// [`crate::ifdb_search::ResolvedGame::record`].
    pending_record: Option<Box<IFiction>>,
    /// A transient status/error line shown under the frame title.
    status: Option<String>,
}

impl SearchModal {
    pub fn new() -> Self {
        SearchModal {
            query: TextField::new(""),
            view: View::Input,
            inflight: None,
            hits: Vec::new(),
            hit_sel: 0,
            seed_hits: Vec::new(),
            showing_seed: false,
            options: Vec::new(),
            opt_sel: 0,
            pending_record: None,
            status: None,
        }
    }

    /// Kick off the "Popular on IFDB" seed query (SQ-0473). Call once, right
    /// after construction; marks the modal busy (its normal "Searching…"
    /// state) and returns the action for the picker to dispatch to the
    /// worker. A failed or empty seed just leaves the modal on its ordinary
    /// empty query box (see `on_event`) — it never blocks opening.
    pub fn open(&mut self) -> ModalAction {
        self.inflight = Some(Inflight::Seed);
        ModalAction::Seed
    }

    /// True while a request is in flight (used to keep the picker's redraw loop
    /// polling for the worker's result).
    pub fn busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// The title of the currently-selected hit, for the "Downloaded X" toast /
    /// progress line the picker shows on success.
    pub fn selected_hit_title(&self) -> Option<&str> {
        self.hits.get(self.hit_sel).map(|h| h.title.as_str())
    }

    /// Consume the iFiction record resolved for the game currently being
    /// downloaded (SQ-0474). The picker calls this exactly once, when
    /// dispatching a `ModalAction::Download` to the worker, so the download
    /// job can populate the sidecar + cover — see `pending_record`.
    pub fn take_pending_record(&mut self) -> Option<Box<IFiction>> {
        self.pending_record.take()
    }

    // ── Key handling ──────────────────────────────────────────────────────

    /// Feed a keypress; returns what the picker should do.
    pub fn on_key(&mut self, code: KeyCode) -> ModalAction {
        // While busy, only Esc responds — it abandons the pending result.
        if self.inflight.is_some() {
            if code == KeyCode::Esc {
                self.inflight = None;
                self.status = Some("Cancelled".to_string());
            }
            return ModalAction::None;
        }
        // A fresh keystroke clears a stale status line.
        self.status = None;
        match self.view {
            View::Input => self.input_key(code),
            View::Results => self.results_key(code),
            View::Choosing => self.choosing_key(code),
        }
    }

    fn input_key(&mut self, code: KeyCode) -> ModalAction {
        match code {
            KeyCode::Esc => ModalAction::Close,
            KeyCode::Enter => {
                let q = self.query.as_str().trim().to_string();
                if q.is_empty() {
                    return ModalAction::None;
                }
                self.inflight = Some(Inflight::Search);
                ModalAction::Search(q)
            }
            KeyCode::Backspace => {
                self.query.backspace();
                ModalAction::None
            }
            KeyCode::Delete => {
                self.query.delete();
                ModalAction::None
            }
            KeyCode::Left => {
                self.query.left();
                ModalAction::None
            }
            KeyCode::Right => {
                self.query.right();
                ModalAction::None
            }
            KeyCode::Home => {
                self.query.home();
                ModalAction::None
            }
            KeyCode::End => {
                self.query.end();
                ModalAction::None
            }
            KeyCode::Char(c) => {
                self.query.insert(c);
                ModalAction::None
            }
            _ => ModalAction::None,
        }
    }

    fn results_key(&mut self, code: KeyCode) -> ModalAction {
        match code {
            // SQ-0473 Esc ladder: from the seed list, close outright (nothing
            // useful behind it); from a typed search's results, return to the
            // cached seed list if there is one, else fall back to the old
            // empty-query behaviour (no seed ever loaded/loaded empty).
            KeyCode::Esc => {
                if self.showing_seed {
                    return ModalAction::Close;
                }
                if !self.seed_hits.is_empty() {
                    self.hits = self.seed_hits.clone();
                    self.hit_sel = 0;
                    self.showing_seed = true;
                } else {
                    self.view = View::Input;
                }
                ModalAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.hit_sel = self.hit_sel.saturating_sub(1);
                ModalAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.hit_sel + 1 < self.hits.len() {
                    self.hit_sel += 1;
                }
                ModalAction::None
            }
            KeyCode::Enter => match self.hits.get(self.hit_sel) {
                Some(hit) => {
                    self.inflight = Some(Inflight::Resolve);
                    ModalAction::Resolve(hit.tuid.clone())
                }
                None => ModalAction::None,
            },
            // Fallback: open the game's IFDB page in a browser.
            KeyCode::Char('o') => match self.hits.get(self.hit_sel).and_then(|h| h.link.clone()) {
                Some(link) => ModalAction::OpenInBrowser(link),
                None => ModalAction::None,
            },
            // SQ-0473: typing over the list (seed or typed-search results)
            // starts a fresh query edit — the list stays visible underneath
            // (see render_body's View::Input arm) until Enter runs the real
            // search, same as typing always has. 'j'/'k'/'o' stay reserved for
            // nav/open as the *first* keystroke (matching this view's existing
            // bindings above); IFDB search is case-insensitive, so a query that
            // starts with one of those letters can still be typed by starting
            // with its capital (e.g. "Jigsaw").
            KeyCode::Char(c) => {
                self.view = View::Input;
                self.input_key(KeyCode::Char(c))
            }
            _ => ModalAction::None,
        }
    }

    fn choosing_key(&mut self, code: KeyCode) -> ModalAction {
        match code {
            // Back to the results list.
            KeyCode::Esc => {
                self.view = View::Results;
                self.options.clear();
                self.opt_sel = 0;
                ModalAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.opt_sel = self.opt_sel.saturating_sub(1);
                ModalAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.opt_sel + 1 < self.options.len() {
                    self.opt_sel += 1;
                }
                ModalAction::None
            }
            KeyCode::Enter => match self.options.get(self.opt_sel) {
                Some(opt) => {
                    self.inflight = Some(Inflight::Download);
                    ModalAction::Download(opt.url.clone())
                }
                None => ModalAction::None,
            },
            _ => ModalAction::None,
        }
    }

    // ── Worker events ─────────────────────────────────────────────────────

    /// Feed a worker event. Returns a follow-up action (e.g. auto-download when
    /// a game has exactly one playable file). A [`SearchEvent::Downloaded`] is
    /// NOT handled here — the picker consumes that directly (it owns the path
    /// and the list refresh) and then closes the modal.
    pub fn on_event(&mut self, ev: &SearchEvent) -> ModalAction {
        match ev {
            SearchEvent::Results(hits) => match self.inflight {
                Some(Inflight::Search) => {
                    self.inflight = None;
                    self.hits = hits.clone();
                    self.hit_sel = 0;
                    self.showing_seed = false;
                    if hits.is_empty() {
                        self.status = Some("No games found".to_string());
                        self.view = View::Input;
                    } else {
                        self.view = View::Results;
                    }
                    ModalAction::None
                }
                Some(Inflight::Seed) => {
                    // SQ-0473: cache the seed list; an empty reply just leaves
                    // the modal on its ordinary empty query box (View::Input,
                    // already the starting state) rather than forcing a status
                    // line for what's an automatic, non-user-initiated load.
                    self.inflight = None;
                    self.seed_hits = hits.clone();
                    if !hits.is_empty() {
                        self.hits = hits.clone();
                        self.hit_sel = 0;
                        self.showing_seed = true;
                        self.view = View::Results;
                    }
                    ModalAction::None
                }
                _ => ModalAction::None, // stale (user backed out, or another job in flight)
            },
            SearchEvent::Options(resolved) => {
                if self.inflight != Some(Inflight::Resolve) {
                    return ModalAction::None; // stale
                }
                self.inflight = None;
                // Cached regardless of how many options came back — a
                // `Download` action, whether auto-fired below or from a
                // later `View::Choosing` pick, always wants this game's
                // record (SQ-0474).
                self.pending_record = resolved.record.clone();
                match resolved.options.len() {
                    0 => {
                        self.status =
                            Some("No directly-playable file — press o to open its IFDB page"
                                .to_string());
                        self.view = View::Results;
                        ModalAction::None
                    }
                    1 => {
                        // Exactly one: download it straight away.
                        self.inflight = Some(Inflight::Download);
                        ModalAction::Download(resolved.options[0].url.clone())
                    }
                    _ => {
                        self.options = resolved.options.clone();
                        self.opt_sel = 0;
                        self.view = View::Choosing;
                        ModalAction::None
                    }
                }
            }
            SearchEvent::Failed(msg) => {
                self.inflight = None;
                self.status = Some(msg.clone());
                // Fall back to a browsable view.
                self.view = if self.hits.is_empty() { View::Input } else { View::Results };
                ModalAction::None
            }
            SearchEvent::Downloaded(_) => ModalAction::None,
        }
    }
}

impl Default for SearchModal {
    fn default() -> Self {
        Self::new()
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Modal dimensions (columns × rows), clamped to the area by `draw_dialog`.
const MODAL_W: u16 = 70;
const MODAL_H: u16 = 20;

/// Draw the modal and return the dialog rects (for click hit-testing). The
/// query field is always drawn in the top content row; the list (hits or
/// download options) fills the rows below, and the final row carries the
/// "Results from IFDB" attribution.
pub fn draw_search_modal(modal: &SearchModal, area: Rect, cs: &ColorScheme, buf: &mut Buffer) -> DialogRects {
    let st = DialogStyle::from_colors(cs);
    let text = cs.theme.get("story_info_value").style;
    let dim = cs.theme.get("story_info_label").style;
    // Reversed, same idiom as every other caret text field in the app (the
    // save-name/text-entry dialogs, the command palette): a plain style with
    // no REVERSED modifier renders no visible caret block at all.
    let caret = st.frame.add_modifier(Modifier::REVERSED);

    let title = match modal.view {
        View::Choosing => "IFDB — choose a file",
        _ => "IFDB search",
    };

    let field = DialogField {
        label: "Search: ",
        value: modal.query.as_str(),
        cursor: modal.query.cursor,
        show_caret: modal.view == View::Input && modal.inflight.is_none(),
        dim: modal.query.is_empty(),
        text_style: text,
        dim_style: dim,
        caret_style: caret,
    };

    let spec = DialogSpec {
        title,
        placement: Placement::Centered { w: MODAL_W, h: MODAL_H },
        buttons: &[],
        show_close: true,
        default: None,
        focus: None,
        field: Some(field),
    };
    let rects = draw_dialog(buf, area, &spec, &st);

    // Content rows below the field: [status?] [list…] [attribution].
    let content = rects.content;
    if content.height <= 1 || content.width == 0 {
        return rects;
    }
    let inner = Rect::new(content.x, content.y + 1, content.width, content.height - 1);
    render_body(modal, inner, cs, buf);
    rects
}

fn render_body(modal: &SearchModal, area: Rect, cs: &ColorScheme, buf: &mut Buffer) {
    let sel_style = cs.theme.get("ifdb_result_selected").style;
    let row_style = cs.theme.get("ifdb_result").style;
    let meta_style = cs.theme.get("ifdb_result_meta").style;
    let marker_style = cs.theme.get("ifdb_download_marker").style;
    let attrib_style = cs.theme.get("ifdb_attribution").style;
    let alert = cs.theme.get("alert").style;

    let mut y = area.y;
    let bottom = area.bottom();

    // Reserve the last row for the "Results from IFDB" attribution.
    let list_bottom = bottom.saturating_sub(1);

    // A status/error line, if present.
    if let Some(status) = &modal.status {
        if y < list_bottom {
            put_str(buf, area.x, y, area.width, status, alert);
            y += 1;
        }
    }

    // Hint line describing the current view's actions. SQ-0473: the "Popular
    // on IFDB" label doubles as the browse-list's own header (reusing the
    // existing meta style — no new selector) and documents the Esc ladder.
    let hint = match modal.view {
        View::Input => "Type a title or author, Enter to search.",
        View::Results if modal.busy() => "Fetching download options…",
        View::Results if modal.showing_seed => {
            "Popular on IFDB · ↑/↓ move · Enter download · o open page · Esc close"
        }
        View::Results if !modal.seed_hits.is_empty() => {
            "↑/↓ move · Enter download · o open page · Esc back to popular"
        }
        View::Results => "↑/↓ move · Enter download · o open page · Esc edit query",
        View::Choosing => "↑/↓ move · Enter download this file · Esc back",
    };
    let busy_note = if modal.busy() && modal.view == View::Input { "Searching…" } else { hint };
    if y < list_bottom {
        put_str(buf, area.x, y, area.width, busy_note, meta_style);
        y += 1;
    }

    match modal.view {
        // SQ-0473: typing over a list keeps it visible underneath (the modal
        // stays on View::Input, editing `query`, while `hits` still holds
        // whatever list — seed or a prior search — was showing).
        View::Input | View::Results => {
            let rows = list_bottom.saturating_sub(y) as usize;
            let start = scroll_start(modal.hit_sel, modal.hits.len(), rows);
            for (i, hit) in modal.hits.iter().enumerate().skip(start).take(rows) {
                let selected = i == modal.hit_sel;
                render_hit_row(buf, area, y, hit, selected, row_style, sel_style, meta_style);
                y += 1;
            }
        }
        View::Choosing => {
            let rows = list_bottom.saturating_sub(y) as usize;
            let start = scroll_start(modal.opt_sel, modal.options.len(), rows);
            for (i, opt) in modal.options.iter().enumerate().skip(start).take(rows) {
                let selected = i == modal.opt_sel;
                let base = if selected { sel_style } else { row_style };
                // The download glyph in its own (accent) style, then the file.
                put_str(buf, area.x, y, 2, "⭳ ", if selected { base } else { marker_style });
                let fmt = opt.format.as_deref().unwrap_or("story file");
                let line = format!("{}  ({})", opt.filename, fmt);
                put_str(buf, area.x + 2, y, area.width.saturating_sub(2), &line, base);
                y += 1;
            }
        }
    }

    // Attribution footer (honours IFDB's CC-BY metadata license).
    put_str(buf, area.x, list_bottom, area.width, "Results from IFDB (ifdb.org)", attrib_style);
}

/// Right-hand gutter reserved so a row's content never touches the dialog's
/// border column.
const ROW_MARGIN: u16 = 1;

/// One result row: "Title — Author", clipped with an ellipsis to leave room for
/// a right-aligned "★rating (year)" tail. The tail's width is reserved FIRST
/// and kept clear of the border gutter, then the title/author span is clipped
/// to end at least one space before it — so long text is truncated, never
/// overdrawn by the tail or bled into the margin. A tail that wouldn't leave
/// room for any title text is dropped rather than overflowing the row.
///
/// Selected rows fill their full width with `base` first (so the highlight
/// reaches both edges, matching the picker's other selected-row lists) and
/// draw the tail in that same style — otherwise it's invisible against the
/// reversed background.
fn render_hit_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    hit: &SearchHit,
    selected: bool,
    row_style: Style,
    sel_style: Style,
    meta_style: Style,
) {
    let base = if selected { sel_style } else { row_style };
    if selected {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(base);
            }
        }
    }

    let content_right = area.right().saturating_sub(ROW_MARGIN);
    let content_w = content_right.saturating_sub(area.x);

    let tail = hit_rating(hit);
    let tail_w = tail.as_ref().map(|t| t.chars().count() as u16).unwrap_or(0);
    // Need the tail's width plus a 1-space gap before it; otherwise the row's
    // too narrow for both — drop the tail (never overflow).
    let (title_w, tail) = if tail_w > 0 && tail_w + 1 < content_w {
        (content_w - tail_w - 1, tail)
    } else {
        (content_w, None)
    };

    let line = format_hit(hit);
    let clipped = clip_with_ellipsis(&line, title_w);
    put_str(buf, area.x, y, title_w, &clipped, base);

    if let Some(t) = tail {
        let tail_style = if selected { base } else { meta_style };
        let tail_x = content_right - tail_w;
        put_str(buf, tail_x, y, tail_w, &t, tail_style);
    }
}

/// Clip `s` to at most `width` characters, appending an ellipsis if it had to
/// be shortened (the ellipsis itself counts against `width`).
fn clip_with_ellipsis(s: &str, width: u16) -> String {
    let width = width as usize;
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut out: String = chars[..width - 1].iter().collect();
    out.push('…');
    out
}

/// "Title — Author" for a hit row.
fn format_hit(hit: &SearchHit) -> String {
    match &hit.author {
        Some(a) => format!("{} — {}", hit.title, a),
        None => hit.title.clone(),
    }
}

/// A compact rating/year tail like "★2.5 (1990)", or just the year, or None.
fn hit_rating(hit: &SearchHit) -> Option<String> {
    match (hit.star_rating, hit.published.as_deref()) {
        (Some(r), Some(y)) => Some(format!("★{r} {y}")),
        (Some(r), None) => Some(format!("★{r}")),
        (None, Some(y)) => Some(format!("({y})")),
        (None, None) => None,
    }
}

/// First index to show so `sel` stays visible in a window of `rows`.
fn scroll_start(sel: usize, len: usize, rows: usize) -> usize {
    if rows == 0 || len <= rows {
        return 0;
    }
    if sel < rows {
        0
    } else {
        (sel + 1 - rows).min(len - rows)
    }
}

/// Write `s` (truncated to `width` cells) at `(x, y)` in `style`.
fn put_str(buf: &mut Buffer, x: u16, y: u16, width: u16, s: &str, style: Style) {
    let end = x.saturating_add(width);
    for (cx, ch) in (x..end).zip(s.chars()) {
        if let Some(cell) = buf.cell_mut((cx, y)) {
            let mut tmp = [0u8; 4];
            cell.set_symbol(ch.encode_utf8(&mut tmp)).set_style(style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifdb_search::{DownloadOption, ResolvedGame, SearchEvent, SearchHit};

    fn hit(tuid: &str, title: &str) -> SearchHit {
        SearchHit {
            tuid: tuid.into(),
            title: title.into(),
            author: Some("Anon".into()),
            link: Some(format!("https://ifdb.org/viewgame?id={tuid}")),
            published: Some("1990".into()),
            star_rating: Some(3.0),
            num_ratings: Some(2),
            has_cover: false,
        }
    }

    fn opt(name: &str) -> DownloadOption {
        DownloadOption {
            filename: name.into(),
            url: format!("https://x/{name}"),
            format: Some("zcode".into()),
        }
    }

    /// A resolved game with no iFiction record — the shape most existing
    /// tests want (they're exercising the options/download plumbing, not
    /// SQ-0474's metadata threading).
    fn resolved(options: Vec<DownloadOption>) -> ResolvedGame {
        ResolvedGame { options, record: None }
    }

    #[test]
    fn typing_then_enter_dispatches_a_search() {
        let mut m = SearchModal::new();
        for c in "zork".chars() {
            assert_eq!(m.on_key(KeyCode::Char(c)), ModalAction::None);
        }
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::Search("zork".into()));
        assert!(m.busy(), "a dispatched search marks the modal busy");
    }

    #[test]
    fn empty_query_enter_does_nothing() {
        let mut m = SearchModal::new();
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::None);
        assert!(!m.busy());
    }

    #[test]
    fn esc_from_input_closes() {
        let mut m = SearchModal::new();
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::Close);
    }

    #[test]
    fn results_arrive_then_enter_resolves_selected() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter); // Search, busy
        let hits = vec![hit("aaa", "Alpha"), hit("bbb", "Beta")];
        assert_eq!(m.on_event(&SearchEvent::Results(hits)), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.selected_hit_title(), Some("Alpha"));
        m.on_key(KeyCode::Down);
        assert_eq!(m.selected_hit_title(), Some("Beta"));
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::Resolve("bbb".into()));
        assert!(m.busy());
    }

    #[test]
    fn empty_results_returns_to_input_with_a_status() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('x'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![]));
        assert!(!m.busy());
        // Back to input; a new keystroke edits the query again.
        assert_eq!(m.on_key(KeyCode::Char('y')), ModalAction::None);
    }

    #[test]
    fn one_download_option_auto_downloads() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter); // Resolve
        let action = m.on_event(&SearchEvent::Options(resolved(vec![opt("a.z5")])));
        assert_eq!(action, ModalAction::Download("https://x/a.z5".into()));
        assert!(m.busy());
    }

    /// SQ-0474: the iFiction record resolved alongside the options is cached
    /// for whichever `Download` follows, and handed to the picker exactly
    /// once — `take_pending_record` doesn't return it twice.
    #[test]
    fn resolved_record_is_cached_until_taken_once() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter); // Resolve
        assert!(m.take_pending_record().is_none(), "nothing resolved yet");

        let record = Box::new(IFiction { title: Some("Alpha".into()), ..Default::default() });
        let action = m.on_event(&SearchEvent::Options(ResolvedGame {
            options: vec![opt("a.z5"), opt("a.z8")], // >1: no auto-download, record still cached
            record: Some(record.clone()),
        }));
        assert_eq!(action, ModalAction::None);
        assert_eq!(m.take_pending_record(), Some(record));
        assert!(m.take_pending_record().is_none(), "taken exactly once");
    }

    #[test]
    fn several_options_open_the_chooser() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter);
        let action = m.on_event(&SearchEvent::Options(resolved(vec![opt("a.z5"), opt("a.z8")])));
        assert_eq!(action, ModalAction::None);
        assert!(!m.busy());
        // Choosing view: Enter downloads the selected option.
        m.on_key(KeyCode::Down);
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::Download("https://x/a.z8".into()));
    }

    #[test]
    fn no_playable_option_offers_the_browser_fallback() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter);
        assert_eq!(m.on_event(&SearchEvent::Options(resolved(vec![]))), ModalAction::None);
        // 'o' opens the IFDB page.
        assert_eq!(
            m.on_key(KeyCode::Char('o')),
            ModalAction::OpenInBrowser("https://ifdb.org/viewgame?id=aaa".into())
        );
    }

    #[test]
    fn esc_in_results_returns_to_query_editing() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::None);
        // Now in input view: Esc closes.
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::Close);
    }

    #[test]
    fn a_failed_event_shows_a_status_and_is_not_busy() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Failed("IFDB unreachable".into()));
        assert!(!m.busy());
        // A new keystroke edits the query.
        assert_eq!(m.on_key(KeyCode::Char('x')), ModalAction::None);
    }

    #[test]
    fn stale_results_after_cancel_are_ignored() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter); // busy, inflight=Search
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::None); // cancel
        assert!(!m.busy());
        // A late result must not jump to the results view.
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert_eq!(m.selected_hit_title(), None, "cancelled search result is dropped");
    }

    #[test]
    fn busy_ignores_typing_but_esc_cancels() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        assert!(m.busy());
        // Typing while busy is ignored.
        assert_eq!(m.on_key(KeyCode::Char('q')), ModalAction::None);
        assert!(m.busy());
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::None);
        assert!(!m.busy(), "Esc abandons the in-flight request");
    }

    #[test]
    fn scroll_start_keeps_selection_visible() {
        assert_eq!(scroll_start(0, 10, 5), 0);
        assert_eq!(scroll_start(4, 10, 5), 0);
        assert_eq!(scroll_start(5, 10, 5), 1);
        assert_eq!(scroll_start(9, 10, 5), 5);
        assert_eq!(scroll_start(3, 3, 5), 0, "list shorter than window starts at 0");
    }

    // ── SQ-0473: seed list on open ──────────────────────────────────────────

    #[test]
    fn open_marks_the_modal_busy_and_asks_for_the_seed() {
        let mut m = SearchModal::new();
        assert!(!m.busy());
        assert_eq!(m.open(), ModalAction::Seed);
        assert!(m.busy(), "open() marks the modal busy, same as a dispatched search");
        assert_eq!(m.view, View::Input, "no seed rows yet — still the empty query box");
    }

    #[test]
    fn seed_results_land_on_the_browsable_seed_list() {
        let mut m = SearchModal::new();
        m.open();
        let seed = vec![hit("pop1", "Popular One"), hit("pop2", "Popular Two")];
        assert_eq!(m.on_event(&SearchEvent::Results(seed.clone())), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Results);
        assert!(m.showing_seed, "results view starts on the seed list");
        assert_eq!(m.selected_hit_title(), Some("Popular One"));
        // Download/resolve/open-in-browser work the same as any other hit row.
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::Resolve("pop1".into()));
    }

    #[test]
    fn empty_seed_result_degrades_to_the_plain_empty_query_box() {
        let mut m = SearchModal::new();
        m.open();
        assert_eq!(m.on_event(&SearchEvent::Results(vec![])), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Input, "nothing to browse — falls back silently");
        assert!(m.status.is_none(), "an empty seed isn't a user-facing error");
    }

    #[test]
    fn seed_fetch_failure_degrades_to_the_empty_query_box_with_a_status() {
        let mut m = SearchModal::new();
        m.open();
        assert_eq!(m.on_event(&SearchEvent::Failed("IFDB unreachable".into())), ModalAction::None);
        assert!(!m.busy());
        assert_eq!(m.view, View::Input, "never blocks opening the modal");
        assert_eq!(m.status.as_deref(), Some("IFDB unreachable"));
    }

    #[test]
    fn esc_from_the_seed_list_closes_the_modal() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("pop1", "Popular One")]));
        assert!(m.showing_seed);
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::Close, "one level shorter than before");
    }

    #[test]
    fn typing_over_the_seed_list_keeps_it_visible_until_enter_searches() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("pop1", "Popular One")]));
        assert_eq!(m.view, View::Results);

        // The first typed character starts editing the query, but the seed
        // rows stay put underneath (still readable off `hits`/`hit_sel`).
        assert_eq!(m.on_key(KeyCode::Char('z')), ModalAction::None);
        assert_eq!(m.view, View::Input);
        assert_eq!(m.hits.len(), 1, "seed rows are not cleared by typing");
        assert_eq!(m.query.as_str(), "z");

        // Enter still runs a real search, same as ever.
        assert_eq!(m.on_key(KeyCode::Enter), ModalAction::Search("z".into()));
        assert!(m.busy());
    }

    #[test]
    fn a_typed_search_replaces_the_seed_list_and_esc_returns_to_it() {
        let mut m = SearchModal::new();
        m.open();
        let seed = vec![hit("pop1", "Popular One")];
        m.on_event(&SearchEvent::Results(seed.clone()));
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter); // busy, inflight = Search

        let searched = vec![hit("zzz", "Zork I"), hit("yyy", "Zork II")];
        assert_eq!(m.on_event(&SearchEvent::Results(searched.clone())), ModalAction::None);
        assert!(!m.showing_seed, "typed results replace the seed list");
        assert_eq!(m.selected_hit_title(), Some("Zork I"));

        // Esc from typed results goes back to the seed list, not an empty box.
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::None);
        assert!(m.showing_seed);
        assert_eq!(m.view, View::Results);
        assert_eq!(m.selected_hit_title(), Some("Popular One"));

        // A second Esc, now back on the seed list, closes.
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::Close);
    }

    #[test]
    fn esc_from_typed_results_with_no_cached_seed_falls_back_to_empty_query() {
        // The seed never loaded (e.g. it failed) — Esc from a typed search's
        // results has nothing to return to, so it falls back to the old
        // pre-SQ-0473 behaviour instead of a phantom "seed" screen.
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        assert!(!m.showing_seed);
        assert!(m.seed_hits.is_empty());
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::None);
        assert_eq!(m.view, View::Input);
        assert_eq!(m.on_key(KeyCode::Esc), ModalAction::Close);
    }

    // ── Render: caret, selected-row meta, long-title layout ─────────────────

    #[test]
    fn query_caret_renders_reversed_mid_string_and_at_end() {
        let mut m = SearchModal::new();
        for c in "zork".chars() {
            m.on_key(KeyCode::Char(c));
        }
        m.on_key(KeyCode::Left); // cursor now mid-string, before the final 'k'

        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        let rects = draw_search_modal(&m, area, &cs, &mut buf);
        let fr = rects.field.expect("field rect recorded");

        let label_len = "Search: ".chars().count() as u16;
        let mid_x = fr.x + label_len + 3; // cursor sits at char index 3 ("zor|k")
        assert!(
            buf.cell((mid_x, fr.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "mid-string caret cell should be reversed"
        );

        m.on_key(KeyCode::Right); // back to end-of-text
        let mut buf2 = Buffer::empty(area);
        draw_search_modal(&m, area, &cs, &mut buf2);
        let end_x = fr.x + label_len + 4;
        assert!(
            buf2.cell((end_x, fr.y)).unwrap().style().add_modifier.contains(Modifier::REVERSED),
            "end-of-text caret cell should be reversed"
        );
    }

    #[test]
    fn selected_result_row_keeps_its_rating_tail_visible() {
        let mut m = SearchModal::new();
        m.open();
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha"), hit("bbb", "Beta")]));
        assert_eq!(m.hit_sel, 0);

        let cs = ColorScheme::terminal_default();
        let sel_style = cs.theme.get("ifdb_result_selected").style;
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        let rects = draw_search_modal(&m, area, &cs, &mut buf);

        // Row 0 (selected: "Alpha") is on the first body row under the field +
        // hint line, i.e. content.y + 2 in buffer coordinates. Find it by
        // scanning for the reversed title cell, then check a rating glyph
        // still appears further right on the same row.
        let title_row_y = (0..area.height)
            .find(|&y| {
                (0..area.width).any(|x| {
                    buf.cell((x, y))
                        .is_some_and(|c| c.symbol() == "A" && c.style().add_modifier.contains(Modifier::REVERSED))
                })
            })
            .expect("selected 'Alpha' row found");
        let row_text: String = (0..area.width)
            .filter_map(|x| buf.cell((x, title_row_y)).map(|c| c.symbol().to_string()))
            .collect();
        assert!(row_text.contains("Alpha"), "selected row still shows its title: {row_text:?}");
        assert!(row_text.contains('★'), "selected row keeps its rating tail: {row_text:?}");
        // The row is highlighted edge-to-edge (the fill loop, not just the
        // glyphs) — checked at the content area's left edge, not the buffer's
        // (which would land on the dialog's own border column).
        // `Cell::set_style` patches onto the dialog's opaque background fill
        // rather than replacing it outright, so check the selected style's own
        // fg/modifiers landed rather than exact equality with the whole cell.
        let fill_style = buf.cell((rects.content.x, title_row_y)).unwrap().style();
        assert_eq!(fill_style.fg, sel_style.fg, "row fill uses the selected style's colour");
        assert!(
            fill_style.add_modifier.contains(Modifier::REVERSED),
            "row fill uses the selected style's reversed video"
        );
    }

    #[test]
    fn long_title_row_ellipsizes_leaves_a_gap_and_keeps_the_tail_off_the_border() {
        let long = hit("aaa", &"A very very very long story title indeed".repeat(3));
        let cs = ColorScheme::terminal_default();
        let area = Rect::new(0, 0, MODAL_W, MODAL_H);
        let mut buf = Buffer::empty(area);
        // Row width available to render_hit_row: MODAL_W minus the dialog's own
        // border/inset. Drive render_hit_row directly — it's the unit under
        // test for the layout fix, independent of the dialog chrome's exact inset.
        let row_area = Rect::new(2, 5, 40, 1);
        let row_style = cs.theme.get("ifdb_result").style;
        let sel_style = cs.theme.get("ifdb_result_selected").style;
        let meta_style = cs.theme.get("ifdb_result_meta").style;
        render_hit_row(&mut buf, row_area, row_area.y, &long, false, row_style, sel_style, meta_style);

        // The reserved right-margin gutter column (border-adjacent) stays
        // blank; everything else is the row's actual content.
        let gutter_x = row_area.right() - 1;
        let content_text: String = (row_area.x..gutter_x)
            .filter_map(|x| buf.cell((x, row_area.y)).map(|c| c.symbol().to_string()))
            .collect();
        let gutter_symbol = buf.cell((gutter_x, row_area.y)).unwrap().symbol().to_string();
        assert_eq!(gutter_symbol, " ", "border-margin column stays blank: {content_text:?}");

        assert!(content_text.contains('…'), "long title is ellipsized: {content_text:?}");
        let tail = hit_rating(&long).expect("rated hit has a tail");
        assert!(
            content_text.ends_with(&tail),
            "tail sits flush at the row's content edge: {content_text:?}"
        );
        let ellipsis_i = content_text.find('…').unwrap();
        let tail_i = content_text.find(&tail).unwrap();
        assert!(
            tail_i > ellipsis_i + 1,
            "at least one space between the ellipsis and the tail: {content_text:?}"
        );
    }

    #[test]
    fn narrow_row_drops_the_tail_instead_of_overflowing() {
        let h = hit("aaa", "Zork");
        let cs = ColorScheme::terminal_default();
        for w in 0..4u16 {
            let area = Rect::new(0, 0, w.max(1), 1);
            let mut buf = Buffer::empty(area);
            let row_area = Rect::new(0, 0, w, 1);
            let row_style = cs.theme.get("ifdb_result").style;
            let sel_style = cs.theme.get("ifdb_result_selected").style;
            let meta_style = cs.theme.get("ifdb_result_meta").style;
            // Must not panic for any degenerate width, selected or not.
            render_hit_row(&mut buf, row_area, 0, &h, false, row_style, sel_style, meta_style);
            render_hit_row(&mut buf, row_area, 0, &h, true, row_style, sel_style, meta_style);
        }
    }
}
