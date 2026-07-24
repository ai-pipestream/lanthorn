//! The story-picker's IFDB search modal (SQ-0413): a three-view state machine
//! (type a query → browse results → optionally pick among a game's download
//! files) plus its renderer. All network work is delegated to
//! [`crate::ifdb_search::SearchWorker`]; this module is the pure UI half — the
//! picker drives it by feeding keys/events in and dispatching the [`ModalAction`]s
//! it hands back. Every state transition is unit-tested without a network.
//!
//! Standing modal conventions (matching the picker's other modals): Esc backs
//! out one level (results → query → close), Enter activates, Up/Down (and j/k)
//! navigate lists. While a request is in flight the modal is "busy": keystrokes
//! are ignored except Esc, which abandons the pending result.

use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::colors::ColorScheme;
use crate::ifdb_search::{DownloadOption, SearchEvent, SearchHit};
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
}

/// The modal's full state.
pub struct SearchModal {
    query: TextField,
    view: View,
    inflight: Option<Inflight>,
    hits: Vec<SearchHit>,
    hit_sel: usize,
    options: Vec<DownloadOption>,
    opt_sel: usize,
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
            options: Vec::new(),
            opt_sel: 0,
            status: None,
        }
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
            // Back to editing the query (keeps the hits until a new search).
            KeyCode::Esc => {
                self.view = View::Input;
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
            SearchEvent::Results(hits) => {
                if self.inflight != Some(Inflight::Search) {
                    return ModalAction::None; // stale (user backed out)
                }
                self.inflight = None;
                self.hits = hits.clone();
                self.hit_sel = 0;
                if hits.is_empty() {
                    self.status = Some("No games found".to_string());
                    self.view = View::Input;
                } else {
                    self.view = View::Results;
                }
                ModalAction::None
            }
            SearchEvent::Options(opts) => {
                if self.inflight != Some(Inflight::Resolve) {
                    return ModalAction::None; // stale
                }
                self.inflight = None;
                match opts.len() {
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
                        ModalAction::Download(opts[0].url.clone())
                    }
                    _ => {
                        self.options = opts.clone();
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
    let caret = cs.theme.get("story_header_active").style;

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

    // Hint line describing the current view's actions.
    let hint = match modal.view {
        View::Input => "Type a title or author, Enter to search.",
        View::Results if modal.busy() => "Fetching download options…",
        View::Results => "↑/↓ move · Enter download · o open page · Esc edit query",
        View::Choosing => "↑/↓ move · Enter download this file · Esc back",
    };
    let busy_note = if modal.busy() && modal.view == View::Input { "Searching…" } else { hint };
    if y < list_bottom {
        put_str(buf, area.x, y, area.width, busy_note, meta_style);
        y += 1;
    }

    match modal.view {
        View::Input => {}
        View::Results => {
            let rows = list_bottom.saturating_sub(y) as usize;
            let start = scroll_start(modal.hit_sel, modal.hits.len(), rows);
            for (i, hit) in modal.hits.iter().enumerate().skip(start).take(rows) {
                let selected = i == modal.hit_sel;
                let base = if selected { sel_style } else { row_style };
                let line = format_hit(hit);
                put_str(buf, area.x, y, area.width, &line, base);
                // A rating/year tail in the meta style, right-aligned (only when
                // the row isn't selected, so the reversed selection stays clean).
                if !selected {
                    if let Some(tail) = hit_rating(hit) {
                        let w = tail.chars().count() as u16 + 1;
                        let tail_x = area.x + area.width.saturating_sub(w);
                        put_str(buf, tail_x, y, w, &tail, meta_style);
                    }
                }
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
    use crate::ifdb_search::{DownloadOption, SearchEvent, SearchHit};

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
        let action = m.on_event(&SearchEvent::Options(vec![opt("a.z5")]));
        assert_eq!(action, ModalAction::Download("https://x/a.z5".into()));
        assert!(m.busy());
    }

    #[test]
    fn several_options_open_the_chooser() {
        let mut m = SearchModal::new();
        m.on_key(KeyCode::Char('z'));
        m.on_key(KeyCode::Enter);
        m.on_event(&SearchEvent::Results(vec![hit("aaa", "Alpha")]));
        m.on_key(KeyCode::Enter);
        let action = m.on_event(&SearchEvent::Options(vec![opt("a.z5"), opt("a.z8")]));
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
        assert_eq!(m.on_event(&SearchEvent::Options(vec![])), ModalAction::None);
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
}
