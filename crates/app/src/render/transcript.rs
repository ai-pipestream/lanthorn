//! GAME pane rendering: status line (top), scrolling transcript (middle), input line (bottom).
//!
//! When `state.show_inventory` is true, a 1-row inventory strip is drawn just
//! above the input line (and below the suggestion line when present), shrinking
//! the transcript area by one row exactly as the suggestion line does.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use zvm::cpu::exec::Machine;
use zvm::screen::StatusRight;

use ratatui::style::Color;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget};

use crate::state::{AppState, Focus, TranscriptFilter, TranscriptKind};
use crate::render::paneframe::{draw_framed, BorderStyle};
use super::draw_str_clipped;

// ── Styles ─────────────────────────────────────────────────────────────────────
//
// Status, normal text, and suggestion styles are read from `state.colors` at
// render time.  The CURSOR style remains a local constant as it is structural
// (REVERSED only, no color content mapped by ColorScheme).

pub const CURSOR_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED);

// ── Pure helpers (testable without Machine) ────────────────────────────────────

/// The field values available to status-bar segment templates for one turn.
pub(crate) struct StatusFields {
    pub location: String,
    pub score: Option<String>,
    pub moves: Option<String>,
    pub time: Option<String>,
    pub turns: String,
    pub title: String,
    pub filter: String,
}

fn status_field_value<'a>(f: &'a StatusFields, name: &str) -> &'a str {
    match name {
        "location" => &f.location,
        "score" => f.score.as_deref().unwrap_or(""),
        "moves" => f.moves.as_deref().unwrap_or(""),
        "time" => f.time.as_deref().unwrap_or(""),
        "turns" => &f.turns,
        "title" => &f.title,
        "filter" => &f.filter,
        _ => "", // unknown token → empty
    }
}

/// Resolve a segment's `{placeholder}` template against `f`.
///
/// Returns `Some(resolved)` for a pure-literal segment or one with at least one
/// non-empty placeholder; returns `None` (hide the segment) when the template
/// contains placeholders that ALL resolve to empty. An unterminated `{` is
/// treated as a literal brace.
pub(crate) fn resolve_placeholders(text: &str, f: &StatusFields) -> Option<String> {
    let mut out = String::new();
    let mut had_placeholder = false;
    let mut any_nonempty = false;
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        if let Some(close) = after.find('}') {
            let name = &after[..close];
            had_placeholder = true;
            let val = status_field_value(f, name);
            if !val.is_empty() {
                any_nonempty = true;
            }
            out.push_str(val);
            rest = &after[close + 1..];
        } else {
            out.push('{');
            rest = after;
        }
    }
    out.push_str(rest);
    if had_placeholder && !any_nonempty {
        None
    } else {
        Some(out)
    }
}

/// Pack visible `(text, style, align)` segments into draw ops `(x_col, text, style)`.
///
/// Left cluster packs from the left edge; right cluster packs flush against the
/// right edge; center cluster centers in the gap between them. Truncation when
/// space runs short: drop the center cluster, then truncate the left cluster to
/// the space before the right cluster, preserving the right cluster (clipped only
/// if it alone exceeds the width). `x_col` is relative to the region's left edge.
pub(crate) fn pack_status_clusters(
    visible: &[(String, ratatui::style::Style, crate::colors::Align)],
    width: usize,
) -> Vec<(u16, String, ratatui::style::Style)> {
    use crate::colors::Align;
    let cw = |s: &str| s.chars().count();
    let pick = |a: Align| -> Vec<&(String, ratatui::style::Style, Align)> {
        visible.iter().filter(|(_, _, sa)| *sa == a).collect()
    };
    let left = pick(Align::Left);
    let center = pick(Align::Center);
    let right = pick(Align::Right);
    let sum = |v: &[&(String, ratatui::style::Style, Align)]| v.iter().map(|(t, _, _)| cw(t)).sum::<usize>();
    let left_w = sum(&left);
    let right_w = sum(&right);
    let center_w = sum(&center);

    let mut ops: Vec<(u16, String, ratatui::style::Style)> = Vec::new();

    // RIGHT cluster: flush right, declared order, clipped to the row.
    let right_start = width.saturating_sub(right_w);
    {
        let mut x = right_start;
        for (t, s, _) in &right {
            let avail = width.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // LEFT cluster: flush left, truncated to the space before the right cluster.
    let left_budget = right_start;
    {
        let mut x = 0usize;
        for (t, s, _) in &left {
            if x >= left_budget { break; }
            let avail = left_budget - x;
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    // CENTER cluster: only when it fits in the gap; otherwise dropped.
    let gap_start = left_w;
    let gap_end = right_start;
    if gap_end > gap_start && center_w <= gap_end - gap_start {
        let mut x = gap_start + (gap_end - gap_start - center_w) / 2;
        for (t, s, _) in &center {
            let avail = gap_end.saturating_sub(x);
            if avail == 0 { break; }
            let txt = truncate_line(t, avail).to_string();
            let adv = cw(&txt);
            ops.push((x as u16, txt, *s));
            x += adv;
        }
    }
    ops
}

/// Return the slice of transcript lines visible in `rows` rows, honouring
/// `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned slice always has ≤ `rows` entries and is ordered oldest-first
/// so the caller can draw them top-to-bottom.
///
/// Note: the renderer now uses `visible_wrapped_lines` which handles word-wrap.
/// This function is retained for unit testing the slice logic in isolation.
#[cfg(test)]
pub(crate) fn visible_lines(
    transcript: &[String],
    rows: usize,
    scroll: u16,
) -> &[String] {
    if rows == 0 || transcript.is_empty() {
        return &[];
    }
    // Total lines available.
    let n = transcript.len();
    // `scroll` offsets the window upward from the bottom.
    let scroll = scroll as usize;
    // The window ends (exclusive) at n - scroll, clamped to [0, n].
    let end = n.saturating_sub(scroll);
    // The window starts at end - rows, clamped to 0.
    let start = end.saturating_sub(rows);
    &transcript[start..end]
}

/// Truncate `line` to at most `width` characters (not bytes).
pub(crate) fn truncate_line(line: &str, width: usize) -> &str {
    // Find the byte position after `width` chars.
    let byte_pos = line
        .char_indices()
        .nth(width)
        .map(|(i, _)| i)
        .unwrap_or(line.len());
    &line[..byte_pos]
}

/// Word-wrap a single logical line into display rows of at most `width` columns.
///
/// - Tries to break at spaces (word-wrap): the line is split at the last space
///   that allows a row of ≤ `width` chars.
/// - Falls back to hard char-break for words longer than `width`.
/// - An empty line produces a single empty string (preserves blank lines).
/// - Zero width returns the line unsplit.
pub(crate) fn wrap_line(line: &str, width: u16) -> Vec<String> {
    let width = width as usize;
    if width == 0 {
        return vec![line.to_string()];
    }
    if line.is_empty() {
        return vec![String::new()];
    }

    let mut rows: Vec<String> = Vec::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        let char_count: usize = remaining.chars().count();
        if char_count <= width {
            rows.push(remaining.to_string());
            break;
        }

        // Collect byte offsets for chars 0..=width (inclusive so we see the boundary char).
        // We want the last space at char index ≤ width to break at.
        let mut last_space_before: Option<usize> = None; // byte offset of last space in 0..width
        let mut byte_at_width: usize = remaining.len();  // byte offset of char #width (the first that doesn't fit)
        for (i, (byte_i, ch)) in remaining.char_indices().enumerate() {
            if i == width {
                byte_at_width = byte_i;
                // If the char right at the boundary is a space, break here
                // (the row is exactly `width` non-space chars).
                if ch == ' ' {
                    last_space_before = Some(byte_i);
                }
                break;
            }
            if ch == ' ' {
                last_space_before = Some(byte_i);
            }
        }

        if let Some(sp) = last_space_before {
            // Break at the space: take everything before it, skip the space.
            rows.push(remaining[..sp].to_string());
            // Advance past the space (sp is a byte offset of ' ', so sp+1 is safe for ASCII ' ').
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
        } else {
            // No space found: hard-break at `width` chars.
            rows.push(remaining[..byte_at_width].to_string());
            remaining = &remaining[byte_at_width..];
        }
    }

    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

/// Columns reserved at the left of a META line for the gutter marker (`▏` + space).
pub(crate) const META_GUTTER: u16 = 2;

/// Expand a slice of logical transcript lines into wrapped display rows, carrying
/// each row's `TranscriptKind`. META lines wrap to `width - META_GUTTER` so the
/// gutter marker has room; STORY lines use the full `width`. `kinds` parallels
/// `transcript`; a missing/short entry defaults to `Story`.
///
/// `styles` parallels `transcript`: each logical line's resolved style is
/// carried onto **every** wrapped row it produces, so a line's style is decided
/// once (on the whole logical line) and never re-derived from a wrapped
/// fragment. A missing/short `styles` entry defaults to `Style::default()`.
pub(crate) fn wrap_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    width: u16,
) -> Vec<(String, TranscriptKind, Style)> {
    transcript
        .iter()
        .enumerate()
        .flat_map(|(i, line)| {
            let kind = kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
            let style = styles.get(i).copied().unwrap_or_default();
            let w = match kind {
                TranscriptKind::Meta | TranscriptKind::Warning => width.saturating_sub(META_GUTTER),
                TranscriptKind::Story | TranscriptKind::Input => width,
            };
            wrap_line(line, w).into_iter().map(move |row| (row, kind, style))
        })
        .collect()
}

/// Return the **wrapped** display rows (with kinds) visible in `rows` rows,
/// honouring `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned vec is ordered oldest-first so the caller can draw top-to-bottom.
/// Returns the visible window of wrapped rows AND the total wrapped-row count
/// (so callers can size a scrollbar without re-wrapping).
pub(crate) fn visible_wrapped_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    rows: usize,
    scroll: u16,
    width: u16,
) -> (Vec<(String, TranscriptKind, Style)>, usize) {
    if rows == 0 || transcript.is_empty() {
        return (Vec::new(), 0);
    }
    let display_rows = wrap_lines_kinded(transcript, kinds, styles, width);
    let n = display_rows.len();
    // Clamp scroll so it never exceeds the top: past `n - rows` the window would
    // otherwise shrink from the bottom, blanking viewport rows.
    let max_scroll = n.saturating_sub(rows);
    let scroll = (scroll as usize).min(max_scroll);
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    (display_rows[start..end].to_vec(), n)
}

/// Draw `text` at `(x, y)` into `buf`, using `base_style` for normal characters
/// and `highlight_style` for every case-insensitive occurrence of `query`.
/// Advances `x` by the number of characters drawn. Does not exceed `clip_area`.
///
/// The implementation builds a lowered copy of `text` alongside a map from
/// lowered-byte-offset to original-byte-offset so that match positions found in
/// the lowered string can be safely mapped back to char boundaries in `text`.
/// This is necessary because `to_lowercase()` is not byte-length-preserving
/// (e.g. Turkish dotted-I U+0130 expands from 2 bytes to 3 bytes), so naive
/// offset reuse can produce non-char-boundary panics.
fn draw_str_highlighted(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    base_style: Style,
    query_lower: &str,
    highlight_style: Style,
    clip_area: ratatui::layout::Rect,
) {
    if query_lower.is_empty() {
        draw_str_clipped(buf, x, y, text, base_style, clip_area);
        return;
    }

    // Build a lowered string and a map: for each source char, record
    // (lowered_byte_start, original_byte_start).  A sentinel entry is appended
    // for the end of both strings.
    let mut tl = String::with_capacity(text.len() + 4);
    let mut map: Vec<(usize, usize)> = Vec::with_capacity(text.chars().count() + 1);
    let mut ob = 0usize;
    for ch in text.chars() {
        map.push((tl.len(), ob));
        for lc in ch.to_lowercase() {
            tl.push(lc);
        }
        ob += ch.len_utf8();
    }
    map.push((tl.len(), text.len())); // sentinel

    // Convert a lowered-byte-offset to the nearest original-byte-offset.
    // For a START offset we round DOWN (largest entry with lowered_start <= L).
    // For an END offset we round UP (smallest entry with lowered_start >= L).
    let orig_start_for = |l: usize| -> usize {
        // Binary search for the largest map entry with .0 <= l.
        let idx = map.partition_point(|&(lb, _)| lb <= l);
        // idx is the first entry AFTER l; step back one.
        let i = if idx > 0 { idx - 1 } else { 0 };
        map[i].1
    };
    let orig_end_for = |l: usize| -> usize {
        // Binary search for the smallest map entry with .0 >= l.
        let idx = map.partition_point(|&(lb, _)| lb < l);
        map[idx].1
    };

    let mut cursor_x = x;
    let mut search_from = 0usize; // byte index into tl

    while search_from < tl.len() {
        if let Some(rel) = tl[search_from..].find(query_lower) {
            let tl_match_start = search_from + rel;
            let tl_match_end   = tl_match_start + query_lower.len();

            // Map lowered offsets back to original char boundaries.
            let orig_ms = orig_start_for(tl_match_start);
            let orig_me = orig_end_for(tl_match_end);
            let orig_ss = orig_start_for(search_from);

            // Draw the non-matching prefix.
            let prefix = &text[orig_ss..orig_ms];
            if !prefix.is_empty() {
                draw_str_clipped(buf, cursor_x, y, prefix, base_style, clip_area);
                cursor_x = cursor_x.saturating_add(prefix.chars().count() as u16);
            }
            // Draw the matching segment.
            let matched = &text[orig_ms..orig_me];
            draw_str_clipped(buf, cursor_x, y, matched, highlight_style, clip_area);
            cursor_x = cursor_x.saturating_add(matched.chars().count() as u16);

            search_from = tl_match_end;
        } else {
            // No more matches: draw the rest with the base style.
            let orig_ss = orig_start_for(search_from);
            draw_str_clipped(buf, cursor_x, y, &text[orig_ss..], base_style, clip_area);
            break;
        }
    }
}

/// Format the input prompt line: `"> " + input`.
pub(crate) fn format_input_line(input: &str) -> String {
    format!("> {}", input)
}

/// Format the autocomplete suggestion bar from a list of candidates and the
/// currently-highlighted index.  Returns an empty string when `suggestions` is
/// empty.  The highlighted entry is wrapped in `[brackets]`; others are plain.
///
/// Example: `north  [northeast]  northwest`
pub(crate) fn format_suggestion_line(suggestions: &[String], highlight_idx: usize) -> String {
    if suggestions.is_empty() {
        return String::new();
    }
    let idx = highlight_idx % suggestions.len();
    suggestions
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i == idx {
                format!("[{}]", w)
            } else {
                w.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Format the inventory strip content: "Inv: item1, item2, ..." truncated to `width`.
///
/// Returns None when the strip should not show (show_inventory is false).
/// When show_inventory is true but no items are available, falls back to a hint.
pub(crate) fn format_inventory_line(
    show_inventory: bool,
    player_obj: Option<u16>,
    inventory_fallback: &[String],
    machine: &zvm::cpu::exec::Machine,
) -> Option<String> {
    if !show_inventory {
        return None;
    }
    // Use live object-tree children when player_obj is locked.
    let items: Vec<String> = if let Some(obj) = player_obj {
        crate::inventory::list_inventory(&machine.mem, obj)
    } else {
        inventory_fallback.to_vec()
    };

    if items.is_empty() {
        if player_obj.is_none() && inventory_fallback.is_empty() {
            // Neither source has data yet.
            Some("Inv: (press i)".to_string())
        } else {
            Some("Inv: (empty)".to_string())
        }
    } else {
        let body = items.join(", ");
        Some(format!("Inv: {}", body))
    }
}

// ── Main render function ───────────────────────────────────────────────────────

/// Render the GAME pane into `buf` within `area`:
///
/// - Top row(s): v3 status line (location left, score/turns or time right), reversed style.
///   When `state.colors.status_header_style != None`, the status line is wrapped in a box
///   (3 rows total: border-top, content, border-bottom).  Falls back to plain when the area
///   is too small.
/// - Middle rows: scrolling transcript from `state.transcript` (newest at bottom).
/// - Bottom row(s): `"> " + state.input`; cursor indicator `_` when `state.focus == Focus::Game`.
///   When `state.colors.input_line_style != None`, the input line is wrapped in a box
///   (3 rows total).  Falls back to plain when the area is too small.
/// Renders the GAME pane. Returns `(scrollbar_drawn, max_scroll)`: whether the
/// transcript drew a scrollbar gutter (so the caller can exclude that column
/// from text selection) and the largest meaningful `transcript_scroll` value.
pub fn render_transcript(machine: &Machine, state: &AppState, area: Rect, buf: &mut Buffer) -> (bool, u16) {
    if area.height == 0 || area.width == 0 {
        return (false, 0);
    }

    let normal_style = state.colors.transcript;

    // ── Determine status and input heights based on border style ─────────────

    let status_style_kind = state.colors.status_header_style;
    let input_style_kind  = state.colors.input_line_style;

    // When boxed, status/input each take 3 rows; fall back to 1 if too small.
    // Gate on "any side present" (base OR per-side) so a style="none" + per-side
    // config (e.g. left/right-only) still boxes and draws its side bars.
    let status_boxed = (status_style_kind != BorderStyle::None || state.colors.status_header_sides.any_on()) && area.height >= 5;
    let input_boxed  = (input_style_kind  != BorderStyle::None || state.colors.input_line_sides.any_on()) && area.height >= 5;
    let status_rows: u16 = if status_boxed { 3 } else { 1 };
    let input_rows:  u16 = if input_boxed  { 3 } else { 1 };

    // ── Top row(s): status line ──────────────────────────────────────────────

    let status_region = Rect::new(area.x, area.y, area.width, status_rows.min(area.height));

    if status_boxed {
        // Draw a pane frame around the status region.
        let frame = draw_framed(buf, status_region, status_style_kind, state.colors.status_header_sides, state.colors.status_header, false);
        // Render status text into the inner content row.
        render_status_content(machine, state, buf, frame.content);
    } else {
        render_status_content(machine, state, buf, status_region);
    }

    if area.height < status_rows + 1 {
        return (false, 0);
    }

    // ── Bottom row(s): input line ─────────────────────────────────────────────

    let input_region_top = area.bottom().saturating_sub(input_rows);
    let input_region = Rect::new(area.x, input_region_top, area.width, input_rows.min(area.height));

    if input_boxed {
        let frame = draw_framed(buf, input_region, input_style_kind, state.colors.input_line_sides, state.colors.input_line, false);
        render_input_content(machine, state, buf, frame.content, normal_style);
    } else {
        render_input_content(machine, state, buf, input_region, normal_style);
    }

    // ── Middle area: transcript + inventory + suggestion ─────────────────────

    let middle_top = area.y + status_rows;
    let middle_bottom = input_region_top;
    if middle_top >= middle_bottom {
        return (false, 0);
    }
    let middle_area = Rect::new(area.x, middle_top, area.width, middle_bottom - middle_top);
    render_middle(machine, state, buf, middle_area, normal_style)
}

/// Draw the status bar into `region`.
///
/// When `state.status_msg` is `Some`, it overrides all segments and renders the
/// transient message left-aligned in the base style. Otherwise each segment in
/// `state.colors.statusbar_layout` is resolved (placeholders substituted, empty
/// ones hidden), styled (base patched with the segment style), and packed into
/// left/center/right clusters.
fn render_status_content(
    machine: &Machine,
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    let base = state.colors.status_bar;
    let status_y = region.y;
    let w = region.width as usize;

    // Fill the row with the base style.
    for x in region.x..region.right() {
        if let Some(cell) = buf.cell_mut((x, status_y)) {
            cell.set_symbol(" ").set_style(base);
        }
    }

    // Transient status message overrides the segments.
    if let Some(msg) = state.status_msg.as_deref() {
        let msg_trunc = truncate_line(msg, w);
        draw_str_clipped(buf, region.x, status_y, msg_trunc, base, region);
        return;
    }

    // Build the field values for this turn.
    let sl = machine.status_line();
    let (score, moves, time) = match sl.right {
        StatusRight::ScoreTurns { score, turns } => (Some(score.to_string()), Some(turns.to_string()), None),
        StatusRight::Time { hours, minutes } => (None, None, Some(format!("{:02}:{:02}", hours, minutes))),
    };
    let filter = match state.transcript_filter {
        TranscriptFilter::Both => String::new(),
        TranscriptFilter::Story => "[filter: story]".to_string(),
        TranscriptFilter::Meta => "[filter: meta]".to_string(),
    };
    let fields = StatusFields {
        location: sl.location,
        score,
        moves,
        time,
        turns: state.turns.to_string(),
        title: state.title.clone(),
        filter,
    };

    // Resolve + style + drop hidden segments.
    let visible: Vec<(String, Style, crate::colors::Align)> = state
        .colors
        .statusbar_layout
        .segments
        .iter()
        .filter_map(|seg| {
            resolve_placeholders(&seg.text, &fields).map(|txt| (txt, base.patch(seg.style), seg.align))
        })
        .collect();

    // Pack into clusters and draw.
    for (x, txt, style) in pack_status_clusters(&visible, w) {
        draw_str_clipped(buf, region.x + x, status_y, &txt, style, region);
    }
}

/// Draw the input prompt (and cursor) into `region` with `normal_style`.
///
/// Hidden during char-input mode (`state.char_mode == true`) because the game
/// is awaiting a single keypress, not a typed line.
fn render_input_content(
    _machine: &Machine,
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
    normal_style: Style,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    // In read_char mode the prompt is meaningless — hide it entirely.
    if state.char_mode {
        return;
    }
    let w = region.width as usize;
    let input_y = region.y;

    // The "> " prompt and the typed text are separately styleable (patched over
    // the normal style, so an unset selector renders identically to before).
    let prompt_style = normal_style.patch(state.colors.input_prompt);
    let text_style = normal_style.patch(state.colors.input_text);
    let prefix = format_input_line(""); // "> "
    let prefix_trunc = truncate_line(&prefix, w);
    draw_str_clipped(buf, region.x, input_y, prefix_trunc, prompt_style, region);
    let text_x = region.x + prefix_trunc.chars().count() as u16;
    let text_w = w.saturating_sub(prefix_trunc.chars().count());
    let input_trunc = truncate_line(&state.input, text_w);
    draw_str_clipped(buf, text_x, input_y, input_trunc, text_style, region);

    if state.focus == Focus::Game && !state.any_overlay_open() {
        let cursor_x = text_x + input_trunc.chars().count() as u16;
        if cursor_x < region.right() {
            if let Some(cell) = buf.cell_mut((cursor_x, input_y)) {
                cell.set_symbol("_").set_style(CURSOR_STYLE);
            }
        }
    }
}

/// Render the middle section: inventory strip, suggestion line (or search hint), transcript body.
/// Returns `(scrollbar_drawn, max_scroll)` — whether a scrollbar gutter was
/// drawn in the rightmost column, and the largest meaningful `transcript_scroll`
/// value (total wrapped rows minus the viewport) so the caller can clamp it.
fn render_middle(
    machine: &Machine,
    state: &AppState,
    buf: &mut Buffer,
    area: Rect,
    _normal_style: Style,
) -> (bool, u16) {
    if area.height == 0 || area.width == 0 {
        return (false, 0);
    }
    let w = area.width as usize;

    // The input_y used by the original code was area.bottom() - 1 of the *full* area;
    // here area is already the middle section, so its bottom is the boundary.
    let middle_bottom = area.bottom(); // exclusive

    // ── Inventory strip: one row above middle_bottom ──────────────────────
    let inv_line = format_inventory_line(
        state.show_inventory,
        state.player_obj,
        &state.inventory_fallback,
        machine,
    );
    let has_inventory = inv_line.is_some();
    let inventory_y = middle_bottom.saturating_sub(1);
    if has_inventory && area.height >= 2 && inventory_y > area.y {
        let inv_text = inv_line.as_deref().unwrap_or("");
        let inv_trunc = truncate_line(inv_text, w);
        let inv_style = state.colors.suggestion;
        draw_str_clipped(buf, area.x, inventory_y, inv_trunc, inv_style, area);
    }

    // ── Suggestion line or search hint: above inventory ──────────────────────
    // When search is active, the search hint replaces the suggestion line.
    let rows_from_bottom = 0u16
        + if has_inventory && area.height >= 2 && inventory_y > area.y { 1 } else { 0 };
    let suggestion_y = middle_bottom.saturating_sub(1 + rows_from_bottom);
    let has_search = state.search_query.is_some();
    let has_suggestions = state.focus == Focus::Game && !state.suggestions.is_empty() && !has_search;

    if has_search && area.height >= 2 && suggestion_y > area.y {
        // Draw the search hint line.
        let q = state.search_query.as_deref().unwrap_or("");
        let match_count = state.search_matches.len();
        let cur_idx = if match_count > 0 { state.search_idx + 1 } else { 0 };
        let key_back = state.config.search.key_back;
        let key_forward = state.config.search.key_forward;
        let hint = format!(
            "search: {}  [{}/{}]  {}:back {}:fwd  Esc:clear",
            q, cur_idx, match_count, key_back, key_forward
        );
        let hint_trunc = truncate_line(&hint, w);
        let hint_style = state.colors.suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, hint_trunc, hint_style, area);
    } else if has_suggestions && area.height >= 2 && suggestion_y > area.y {
        let sug_line = format_suggestion_line(&state.suggestions, state.suggestion_idx);
        let sug_trunc = truncate_line(&sug_line, w);
        let sug_style = state.colors.suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, sug_trunc, sug_style, area);
    }

    // ── Transcript body ───────────────────────────────────────────────────────
    if area.height < 2 {
        // Not enough room for transcript when there's an inventory/suggestion row.
        return (false, 0);
    }

    let transcript_top = area.y;
    let transcript_bottom = if (has_search || has_suggestions) && suggestion_y > area.y {
        suggestion_y
    } else if has_inventory && area.height >= 2 && inventory_y > area.y {
        inventory_y
    } else {
        middle_bottom
    };

    if transcript_top >= transcript_bottom {
        return (false, 0);
    }
    let transcript_rows = (transcript_bottom - transcript_top) as usize;

    let visible_indices = state.visible_transcript_indices();
    let filtered_lines: Vec<String> = visible_indices.iter().map(|&i| state.transcript[i].clone()).collect();
    let filtered_kinds: Vec<TranscriptKind> = visible_indices.iter().map(|&i| state.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story)).collect();
    // Resolve each logical line's text style ONCE, before wrapping. Story lines
    // run through the rule list (user → location → system → base); the other
    // kinds use their fixed per-category style. Resolving here (not per wrapped
    // fragment) keeps whole-line matching correct when a line wraps.
    let room_name = state.current_room_name.as_deref();
    let filtered_styles: Vec<Style> = visible_indices
        .iter()
        .zip(filtered_kinds.iter())
        .map(|(&i, kind)| match kind {
            TranscriptKind::Story   => state.colors.resolve_story_style(&state.transcript[i], room_name),
            TranscriptKind::Input   => state.colors.transcript_input,
            TranscriptKind::Meta    => state.colors.transcript_meta,
            TranscriptKind::Warning => state.colors.transcript_warning,
        })
        .collect();
    // Reserve the rightmost column of the body as the scrollbar gutter so text
    // never collides with the scrollbar. Wrap and clip to this narrower body.
    let body_area = if area.width >= 2 {
        Rect { width: area.width - 1, ..area }
    } else {
        area
    };
    let (lines, total_rows) = visible_wrapped_lines_kinded(
        &filtered_lines,
        &filtered_kinds,
        &filtered_styles,
        transcript_rows,
        state.transcript_scroll,
        body_area.width,
    );
    // Search highlight style: black text on yellow background.
    let search_highlight_style = Style::new().fg(Color::Black).bg(Color::Yellow);
    let query_lower = state.search_query.as_deref().map(|q| q.to_lowercase()).unwrap_or_default();

    for (i, (line, kind, style)) in lines.iter().enumerate() {
        let row_y = transcript_top + i as u16;
        if row_y >= transcript_bottom {
            break;
        }
        // Meta/Warning reserve the 2-col gutter and draw their marker glyph;
        // Story/Input draw flush left. The text style was resolved per logical
        // line above and is carried on every wrapped row.
        let (gutter, marker_style) = match kind {
            TranscriptKind::Meta    => (Some(state.symbols.meta_gutter), state.colors.meta_marker),
            TranscriptKind::Warning => (Some(state.symbols.warning_gutter), state.colors.warning_marker),
            TranscriptKind::Story | TranscriptKind::Input => (None, Style::default()),
        };
        let text_x = if let Some(glyph) = gutter {
            draw_str_clipped(buf, body_area.x, row_y, &glyph.to_string(), marker_style, body_area);
            body_area.x + META_GUTTER
        } else {
            body_area.x
        };
        if has_search {
            draw_str_highlighted(buf, text_x, row_y, line, *style, &query_lower, search_highlight_style, body_area);
        } else {
            draw_str_clipped(buf, text_x, row_y, line, *style, body_area);
        }
    }

    // ── Scrollbar (only when the content overflows the viewport) ──────────────
    let drew_scrollbar = total_rows > transcript_rows && area.width >= 2 && transcript_bottom > transcript_top;
    if drew_scrollbar {
        let start = total_rows
            .saturating_sub(state.transcript_scroll as usize)
            .saturating_sub(transcript_rows);
        let sb_area = Rect {
            x: area.right() - 1,
            y: transcript_top,
            width: 1,
            height: transcript_bottom - transcript_top,
        };
        let mut sb_state = ScrollbarState::new(total_rows)
            .viewport_content_length(transcript_rows)
            .position(start);
        StatefulWidget::render(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None),
            sb_area,
            buf,
            &mut sb_state,
        );
    }
    let max_scroll = total_rows.saturating_sub(transcript_rows).min(u16::MAX as usize) as u16;
    (drew_scrollbar, max_scroll)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    // ── Pure helper tests (no Machine required) ──────────────────────────────

    #[test]
    fn input_uses_full_width_warning_wraps_like_meta() {
        let line = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        // Input: full width 8 (no gutter) → unsplit.
        let i = wrap_lines_kinded(&line, &[TranscriptKind::Input], &st, 8);
        assert_eq!(i.iter().map(|(s, _, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
        // Warning: wraps to width-2 = 6 like Meta.
        let w = wrap_lines_kinded(&line, &[TranscriptKind::Warning], &st, 8);
        assert_eq!(w.iter().map(|(s, _, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdef", "gh"]);
    }

    fn fields_score() -> StatusFields {
        StatusFields {
            location: "West of House".into(),
            score: Some("10".into()),
            moves: Some("5".into()),
            time: None,
            turns: "7".into(),
            title: "Zork".into(),
            filter: String::new(),
        }
    }

    #[test]
    fn resolve_placeholders_substitutes_and_hides() {
        let f = fields_score();
        assert_eq!(resolve_placeholders("Score: {score}  Moves: {moves}", &f).as_deref(), Some("Score: 10  Moves: 5"));
        // pure literal always shown
        assert_eq!(resolve_placeholders(" | ", &f).as_deref(), Some(" | "));
        // all-empty placeholder segment hides (time is None on a score game)
        assert_eq!(resolve_placeholders("{time}", &f), None);
        // mixed: one empty, one non-empty placeholder → shown
        assert_eq!(resolve_placeholders("{time}{location}", &f).as_deref(), Some("West of House"));
        // unknown token → empty; all-empty → hidden
        assert_eq!(resolve_placeholders("{bogus}", &f), None);
        // turns vs moves are distinct
        assert_eq!(resolve_placeholders("{turns}/{moves}", &f).as_deref(), Some("7/5"));
    }

    #[test]
    fn pack_clusters_positions_and_truncates() {
        use crate::colors::Align;
        let s = Style::default();
        let mk = |t: &str, a: Align| (t.to_string(), s, a);
        // width 30: left "abc"(0), right "XY"(28)
        let ops = pack_status_clusters(&[mk("abc", Align::Left), mk("XY", Align::Right)], 30);
        let left = ops.iter().find(|(_, t, _)| t == "abc").unwrap();
        let right = ops.iter().find(|(_, t, _)| t == "XY").unwrap();
        assert_eq!(left.0, 0);
        assert_eq!(right.0, 28); // 30 - 2
        // center centered in the gap between left end (3) and right start (28): gap 25, center "cc"(2) at 3 + (25-2)/2 = 14
        let ops2 = pack_status_clusters(&[mk("abc", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 30);
        let center = ops2.iter().find(|(_, t, _)| t == "cc").unwrap();
        assert_eq!(center.0, 14);
        // narrow width 6: right "XY" preserved at 4; center dropped; left "abcdef" truncated to 4 ("abcd")
        let ops3 = pack_status_clusters(&[mk("abcdef", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 6);
        assert!(ops3.iter().all(|(_, t, _)| t != "cc"), "center dropped under pressure");
        let right3 = ops3.iter().find(|(x, _, _)| *x == 4).unwrap();
        assert_eq!(right3.1, "XY");
        let left3 = ops3.iter().find(|(x, _, _)| *x == 0).unwrap();
        assert_eq!(left3.1, "abcd"); // truncated to the 4 cols before the right cluster
    }

    #[test]
    fn render_status_default_bar_matches_today() {
        // With no custom [statusbar], the bar shows location left and the filter
        // indicator right; score/moves come from the (empty) minimal machine.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript_filter = crate::state::TranscriptFilter::Story;

        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        let row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        // filter indicator pinned right (default bar includes ` {filter}`).
        assert!(row.contains("[filter: story]"), "default bar must show the filter indicator: {:?}", row);
        // status row keeps the reversed-video base fill.
        assert!(buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn visible_lines_newest_at_bottom() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 0 → last 5 lines: line5..line9
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[4], "line 9");
        assert_eq!(vis[0], "line 5");
    }

    #[test]
    fn visible_lines_scroll_up() {
        let transcript: Vec<String> = (0..10).map(|i| format!("line {}", i)).collect();
        // 5 rows, scroll 2 → lines 3..7 (end = 10-2=8, start = 8-5=3)
        let vis = visible_lines(&transcript, 5, 2);
        assert_eq!(vis.len(), 5);
        assert_eq!(vis[0], "line 3");
        assert_eq!(vis[4], "line 7");
    }

    #[test]
    fn visible_lines_fewer_than_rows() {
        let transcript = vec!["only one".to_string()];
        let vis = visible_lines(&transcript, 5, 0);
        assert_eq!(vis.len(), 1);
        assert_eq!(vis[0], "only one");
    }

    #[test]
    fn truncate_line_clips_at_width() {
        assert_eq!(truncate_line("hello world", 5), "hello");
        assert_eq!(truncate_line("hi", 10), "hi");
        assert_eq!(truncate_line("abc", 3), "abc");
    }

    #[test]
    fn wrap_line_basic_word_wrap() {
        // "the quick brown fox" at width 9: "the quick" + "brown fox"
        let result = wrap_line("the quick brown fox", 9);
        assert_eq!(result, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_line_hard_break_long_word() {
        // "abcdefghij" at width 4: "abcd" + "efgh" + "ij"
        let result = wrap_line("abcdefghij", 4);
        assert_eq!(result, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_line_fits_in_one_row() {
        let result = wrap_line("hello", 10);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn wrap_line_empty_string() {
        let result = wrap_line("", 10);
        assert_eq!(result, vec![""]);
    }

    #[test]
    fn wrap_line_exact_width() {
        // "abc" at width 3: exactly fits
        let result = wrap_line("abc", 3);
        assert_eq!(result, vec!["abc"]);
    }

    #[test]
    fn wrap_lines_kinded_expands_multiple_logical_lines() {
        let lines = vec![
            "hello world test".to_string(),
            "short".to_string(),
        ];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        // width 5: "hello" + "world" + "test" + "short"
        let result = wrap_lines_kinded(&lines, &kinds, &styles, 5);
        let rows: Vec<&str> = result.iter().map(|(s, _, _)| s.as_str()).collect();
        assert_eq!(rows, vec!["hello", "world", "test", "short"]);
    }

    #[test]
    fn meta_lines_wrap_narrower_and_carry_kind() {
        // An 8-char wordless line: STORY fits in width 8; META wraps to width-2 = 6.
        let transcript = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        let m = wrap_lines_kinded(&transcript, &[TranscriptKind::Meta], &st, 8);
        assert_eq!(m.iter().map(|(s, _, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdef", "gh"]);
        assert!(m.iter().all(|(_, k, _)| matches!(k, TranscriptKind::Meta)));
        let s = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &st, 8);
        assert_eq!(s.iter().map(|(s, _, _)| s.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
    }

    #[test]
    fn wrap_carries_logical_line_style_onto_every_row() {
        use ratatui::style::Color;
        // One logical line that wraps to 3 rows; its style must appear on all rows.
        let transcript = vec!["alpha beta gamma".to_string()];
        let styles = [Style::new().fg(Color::Magenta)];
        let rows = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &styles, 5);
        assert!(rows.len() >= 3, "expected wrap to >= 3 rows, got {}", rows.len());
        assert!(rows.iter().all(|(_, _, s)| s.fg == Some(Color::Magenta)),
            "every wrapped row must carry the logical line's style");
    }

    #[test]
    fn visible_wrapped_lines_kinded_newest_at_bottom() {
        // 3 logical lines at width 10 = 3 display rows; scroll=0, rows=3
        let transcript = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let (vis, total) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, 3, 0, 10);
        assert_eq!(vis.len(), 3);
        assert_eq!(total, 3, "total wrapped rows reported");
        assert_eq!(vis[2].0, "ghi");
    }

    #[test]
    fn visible_wrapped_lines_kinded_scroll_offset() {
        // "hello world" wraps to ["hello", "world"] at width 5
        let transcript = vec!["hello world".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        // scroll=1: end = 2-1=1, start = 1-1=0 → ["hello"]
        let (vis, total) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, 1, 1, 5);
        assert_eq!(vis[0].0, "hello");
        assert_eq!(total, 2, "both wrapped rows counted");
        // scroll=0: end=2, start=1 → ["world"]
        let (vis2, _) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, 1, 0, 5);
        assert_eq!(vis2[0].0, "world");
    }

    #[test]
    fn visible_wrapped_lines_kinded_over_scroll_clamps_to_top() {
        // 5 logical lines, viewport 3 rows. Over-scrolling past the top must
        // keep showing the TOP 3 lines, not shrink the window from the bottom.
        let transcript: Vec<String> = (0..5).map(|i| format!("L{}", i)).collect();
        let kinds = vec![TranscriptKind::Story; 5];
        let styles = vec![Style::default(); 5];
        let (vis, total) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, 3, 999, 10);
        assert_eq!(total, 5);
        assert_eq!(vis.len(), 3, "over-scroll still fills the viewport");
        assert_eq!(vis[0].0, "L0", "top line stays at the top");
        assert_eq!(vis[2].0, "L2");
    }

    #[test]
    fn format_input_line_prefix() {
        assert_eq!(format_input_line("open mailbox"), "> open mailbox");
        assert_eq!(format_input_line(""), "> ");
    }

    // ── Render tests: transcript + input rows (no Machine) ───────────────────
    //
    // We still need a Machine for render_transcript. We build a minimal one from
    // zvm's sample_story (v3) to avoid needing a real fixture file.

    fn minimal_machine() -> Machine {
        use zvm::memory::Memory;
        // Use the same sample_story helper that zvm's own tests use.
        // It's in zvm::header::tests_support but that's cfg(test)-only.
        // Instead we build a minimal valid v3 story buffer ourselves.
        //
        // Minimum valid v3 story file:
        //   byte 0x00 = version (3)
        //   bytes 0x04-0x05 = high memory base (e.g. 0x0040)
        //   bytes 0x06-0x07 = initial PC (e.g. 0x0040)
        //   bytes 0x0A-0x0B = dictionary base (0x0080)
        //   bytes 0x0C-0x0D = object table base (0x0100)
        //   bytes 0x0E-0x0F = global var table base (0x0300)
        //   bytes 0x08-0x09 = static mem base (0x0400)
        //   bytes 0x02-0x03 = (release number, ignored)
        //   Total: 0x500 bytes should be enough.
        //
        // We use the same layout as zvm/src/header.rs tests_support::sample_story(3).
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 3;                       // version = 3
        // high_mem_base = 0x0040
        buf[0x04] = 0x00; buf[0x05] = 0x40;
        // initial_pc = 0x0040 (will contain a QUIT/quit opcode: 0x00 = rtrue? use 0xba = quit)
        buf[0x06] = 0x00; buf[0x07] = 0x40;
        // dict base = 0x0080
        buf[0x0A] = 0x00; buf[0x0B] = 0x80;
        // object table = 0x0100
        buf[0x0C] = 0x01; buf[0x0D] = 0x00;
        // global var table = 0x0300
        buf[0x0E] = 0x03; buf[0x0F] = 0x00;
        // static mem base = 0x0400 (dynamic = 0x0000..0x03FF)
        buf[0x08] = 0x04; buf[0x09] = 0x00;
        // abbreviation table = 0x0060 (word at 0x18-0x19)
        buf[0x18] = 0x00; buf[0x19] = 0x60;

        // Place a valid dictionary at 0x0080: word-separators count=0, entry_size=4, entry_count=0.
        buf[0x0080] = 0; // 0 word-separators
        buf[0x0081] = 4; // entry size = 4 bytes
        buf[0x0082] = 0; buf[0x0083] = 0; // entry count = 0

        // Object table at 0x0100: 31 prop-default words (62 bytes), then no objects.
        // Property defaults: all zero (62 bytes, already 0).

        // Put a QUIT opcode at 0x0040 so stepping won't panic.
        buf[0x0040] = 0xba; // opcode for 'quit' in v3 (0OP:0x0a → encoded as 0xba).

        let mem = Memory::new(buf).expect("minimal v3 story should be valid");
        Machine::new(mem)
    }

    #[test]
    fn render_kinds_draw_their_own_styles_and_gutters() {
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript_kind("> go north", TranscriptKind::Input);
        state.push_transcript_kind("app message", TranscriptKind::Meta);
        state.push_transcript_kind("VAR 0x15 unimplemented", TranscriptKind::Warning);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Find the row index (1..8) for each tagged line by its first glyph / content.
        let row_text = |y: u16| -> String {
            (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
        };
        // Locate the warning row by its gutter glyph '!' in column 0.
        let warn_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("!"))
            .expect("warning gutter '!' must appear in column 0");
        // Warning gutter cell uses warning_marker (Yellow).
        assert_eq!(buf.cell((0, warn_y)).unwrap().style().fg, Some(Color::Yellow));
        // Warning text is indented past the 2-col gutter and uses transcript_warning (Yellow).
        assert_eq!(buf.cell((2, warn_y)).unwrap().style().fg, Some(Color::Yellow));

        // Meta row: gutter glyph '▏' in column 0.
        let meta_y = (1u16..9).find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("▏"))
            .expect("meta gutter '▏' must appear in column 0");
        assert_eq!(buf.cell((2, meta_y)).unwrap().style().fg, Some(Color::DarkGray)); // transcript_meta

        // Input row: no gutter (text at column 0), cyan fg.
        let input_y = (1u16..9).find(|&y| row_text(y).starts_with("> go north"))
            .expect("input line must render at column 0");
        assert_eq!(buf.cell((0, input_y)).unwrap().style().fg, Some(Color::Cyan)); // transcript_input
    }

    #[test]
    fn wrapped_system_line_keeps_style_on_all_rows() {
        // Regression: a bracketed system line long enough to wrap must keep its
        // transcript:system style on EVERY wrapped row. The style is resolved on
        // the whole logical line, not the wrapped fragments (neither of which is
        // itself fully bracketed).
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript("[Your score just went up by ten points.]"); // Story
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 20, 12); // narrow → the 40-char line wraps
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        let mut checked = 0;
        for y in 1u16..10 {
            let c0 = buf.cell((0, y)).expect("cell");
            if c0.symbol().trim().is_empty() {
                continue; // blank row (not part of the wrapped line)
            }
            assert_eq!(
                c0.style().fg,
                Some(Color::DarkGray),
                "row {} must carry transcript:system (DarkGray) on a wrapped system line",
                y
            );
            checked += 1;
        }
        assert!(checked >= 2, "system line should wrap to >= 2 rows; checked {}", checked);
    }

    #[test]
    fn render_story_location_line_is_bold() {
        use ratatui::style::Modifier;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.current_room_name = Some("West of House".to_string());
        state.push_transcript("West of House"); // Story
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        let y = (1u16..9).find(|&y| {
            let row: String = (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
            row.starts_with("West of House")
        }).expect("location line must render");
        assert!(buf.cell((0, y)).unwrap().modifier.contains(Modifier::BOLD), "location header must be bold");
    }

    #[test]
    fn render_transcript_input_and_transcript_lines() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = vec![
            "You are in a hall.".to_string(),
            "It is dark.".to_string(),
        ];
        state.input = "open mailbox".to_string();
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Bottom row (y=9) should contain "> open mailbox".
        let bottom_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("> open mailbox"),
            "bottom row should contain '> open mailbox'; got: {:?}",
            bottom_row
        );

        // A middle row should contain one of the transcript lines.
        let found_transcript = (1u16..9u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("You are in a hall.") || row.contains("It is dark.")
        });
        assert!(found_transcript, "a middle row should contain a transcript line");
    }

    #[test]
    fn render_transcript_cursor_shown_when_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "hi".to_string();
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Cursor at position 4 ("> hi" = 4 chars → cursor at x=4).
        let cursor_cell = buf.cell((4, 4)).expect("cursor cell should exist");
        assert_eq!(cursor_cell.symbol(), "_", "cursor should be '_' at end of input");
        assert!(
            cursor_cell.modifier.contains(Modifier::REVERSED),
            "cursor cell should have REVERSED modifier"
        );
    }

    #[test]
    fn render_transcript_no_cursor_when_not_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "hi".to_string();
        state.focus = Focus::Map; // not focused on game

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Position x=4 should not have '_'.
        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert_ne!(cell.symbol(), "_", "no cursor when focus is Map");
    }

    #[test]
    fn render_transcript_no_cursor_when_overlay_open() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "hi".to_string();
        state.focus = Focus::Game; // focused on game, but overlay is open

        // Open the hotkey dialog — the simplest boolean overlay.
        state.hotkey_dialog = true;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Position x=4 (after "> hi") should NOT have '_' because an overlay is open.
        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert_ne!(
            cell.symbol(),
            "_",
            "cursor must be suppressed when an overlay is open even if focus is Game"
        );
        assert!(
            !cell.modifier.contains(Modifier::REVERSED),
            "cursor REVERSED modifier must be absent when overlay is open"
        );
    }

    #[test]
    fn render_transcript_status_line_reversed() {
        let machine = minimal_machine();
        let state = AppState::default();

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Top row (y=0) should all have REVERSED modifier (status line background).
        let top_cell = buf.cell((0, 0)).expect("top-left cell should exist");
        assert!(
            top_cell.modifier.contains(Modifier::REVERSED),
            "top row should have REVERSED modifier for status line"
        );
    }

    #[test]
    fn render_transcript_applies_input_text_and_prompt_styles() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input = "zq".to_string();
        state.colors.input_prompt = Style::new().fg(Color::Green);
        state.colors.input_text = Style::new().fg(Color::Red);

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Find the '>' prompt cell and the typed 'z' cell; check their fg.
        let mut prompt_fg = None;
        let mut text_fg = None;
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(c) = buf.cell((x, y)) {
                    match c.symbol() {
                        ">" if prompt_fg.is_none() => prompt_fg = Some(c.fg),
                        "z" if text_fg.is_none() => text_fg = Some(c.fg),
                        _ => {}
                    }
                }
            }
        }
        assert_eq!(prompt_fg, Some(Color::Green), "'>' uses input_prompt style");
        assert_eq!(text_fg, Some(Color::Red), "typed text uses input_text style");
    }

    #[test]
    fn render_transcript_shows_scrollbar_when_overflowing() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // Far more lines than the viewport → scrollbar must appear.
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // The scrollbar gutter is the rightmost column; its thumb glyph is '█'.
        let gutter: String = (0..area.height)
            .map(|y| buf.cell((area.width - 1, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(gutter.contains('█'), "scrollbar thumb should render in the rightmost column: {:?}", gutter);
    }

    #[test]
    fn render_transcript_no_scrollbar_when_fits() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = vec!["only one line".to_string()];

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        let gutter: String = (0..area.height)
            .map(|y| buf.cell((area.width - 1, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!gutter.contains('█'), "no scrollbar when content fits: {:?}", gutter);
    }

    #[test]
    fn render_transcript_scroll_offset() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // 10 lines, scroll=5 should show lines 0..4 (end=10-5=5, start=5-4=1 for 4-row middle)
        state.transcript = (0..10).map(|i| format!("L{}", i)).collect();
        state.transcript_scroll = 5;

        // 7-row area: 1 status + 5 transcript + 1 input
        let area = Rect::new(0, 0, 40, 7);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Middle rows y=1..5: should NOT show L9 (newest) but should show L4 or earlier.
        let found_l9 = (1u16..6u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("L9")
        });
        assert!(!found_l9, "L9 (newest) should not be visible when scrolled back 5");
    }

    // ── Status line test with czech.z5 fixture (skipped if absent) ───────────

    // ── format_suggestion_line tests ─────────────────────────────────────────

    #[test]
    fn format_suggestion_line_empty() {
        assert_eq!(format_suggestion_line(&[], 0), "");
    }

    #[test]
    fn format_suggestion_line_single_highlighted() {
        let sug = vec!["north".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert_eq!(line, "[north]");
    }

    #[test]
    fn format_suggestion_line_highlight_first() {
        let sug = vec!["north".to_string(), "northeast".to_string(), "northwest".to_string()];
        let line = format_suggestion_line(&sug, 0);
        assert!(line.starts_with("[north]"), "first entry should be highlighted: {}", line);
        assert!(line.contains("northeast") && !line.contains("[northeast]"));
    }

    #[test]
    fn format_suggestion_line_highlight_second() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        let line = format_suggestion_line(&sug, 1);
        assert!(line.contains("[northeast]"), "second entry highlighted: {}", line);
        assert!(!line.contains("[north]"), "first not highlighted: {}", line);
    }

    #[test]
    fn format_suggestion_line_idx_wraps() {
        let sug = vec!["north".to_string(), "northeast".to_string()];
        // idx=2 wraps to 0
        let line = format_suggestion_line(&sug, 2);
        assert!(line.starts_with("[north]"), "idx wraps: {}", line);
    }

    #[test]
    fn render_transcript_shows_suggestion_line_above_input() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input = "nor".to_string();
        state.suggestions = vec!["north".to_string()];
        state.suggestion_idx = 0;

        // 10-row area: row 0=status, rows 1..7=transcript, row 8=suggestion, row 9=input
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Row 9 (bottom) must contain the input.
        let input_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(input_row.contains("> nor"), "input row: {:?}", input_row);

        // Row 8 must contain the suggestion.
        let sug_row: String = (0..40u16)
            .map(|x| buf.cell((x, 8)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(sug_row.contains("north"), "suggestion row: {:?}", sug_row);
    }

    #[test]
    fn render_transcript_status_line_nonblank_with_fixture() {
        let fixture = std::path::Path::new(
            "/Volumes/Videos/Source/babelmap/crates/zvm/tests/fixtures/czech.z5",
        );
        if !fixture.exists() {
            eprintln!("SKIP: czech.z5 fixture not found");
            return;
        }

        let data = std::fs::read(fixture).expect("read czech.z5");
        let mem = zvm::memory::Memory::new(data).expect("parse czech.z5");
        let machine = Machine::new(mem);

        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Top row should have at least one non-space character (status line text or reversed bg).
        // We verify the REVERSED modifier is present on the whole row.
        let top_has_reversed = (0..80u16).all(|x| {
            buf.cell((x, 0))
                .map(|c| c.modifier.contains(Modifier::REVERSED))
                .unwrap_or(false)
        });
        assert!(
            top_has_reversed,
            "status line row should be fully reversed-video for czech.z5"
        );
    }

    // ── Inventory strip tests ─────────────────────────────────────────────────

    #[test]
    fn render_transcript_inventory_strip_shown_when_enabled() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inventory_fallback = vec!["brass lamp".to_string(), "sword".to_string()];

        // 10-row area: row0=status, rows1..7=transcript, row8=inventory, row9=input.
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Row 9 (bottom) must contain the input prompt.
        let input_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(input_row.contains("> "), "input row: {:?}", input_row);

        // Row 8 must contain the inventory strip.
        let inv_row: String = (0..40u16)
            .map(|x| buf.cell((x, 8)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(inv_row.contains("Inv:"), "inventory row should contain 'Inv:': {:?}", inv_row);
        assert!(inv_row.contains("brass lamp"), "inventory row should contain item: {:?}", inv_row);
    }

    #[test]
    fn render_transcript_inventory_strip_hidden_when_disabled() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.show_inventory = false;
        state.inventory_fallback = vec!["brass lamp".to_string()];

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // No row should contain "Inv:".
        let found_inv = (0..10u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("Inv:")
        });
        assert!(!found_inv, "Inv: should not appear when show_inventory is false");
    }

    #[test]
    fn render_transcript_inventory_strip_shrinks_transcript_area() {
        // With inventory strip shown, transcript should have one fewer row.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = (0..10).map(|i| format!("L{}", i)).collect();
        state.inventory_fallback = vec!["lamp".to_string()];

        // 7-row area without inventory: 1 status + 5 transcript + 1 input.
        // 7-row area with inventory:    1 status + 4 transcript + 1 inventory + 1 input.
        let area = Rect::new(0, 0, 40, 7);

        // Without inventory.
        let mut buf_no = Buffer::empty(area);
        state.show_inventory = false;
        render_transcript(&machine, &state, area, &mut buf_no);
        let transcript_rows_no = (1u16..6u16).filter(|&y| {
            let row: String = (0..40u16)
                .map(|x| buf_no.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.trim_end().contains('L')
        }).count();

        // With inventory.
        let mut buf_yes = Buffer::empty(area);
        state.show_inventory = true;
        render_transcript(&machine, &state, area, &mut buf_yes);
        let transcript_rows_yes = (1u16..6u16).filter(|&y| {
            let row: String = (0..40u16)
                .map(|x| buf_yes.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.trim_end().contains('L')
        }).count();

        assert!(
            transcript_rows_yes < transcript_rows_no,
            "transcript should have fewer rows with inventory strip ({} vs {})",
            transcript_rows_yes, transcript_rows_no
        );
    }

    #[test]
    fn format_inventory_line_live_vs_fallback_vs_hint() {
        let machine = minimal_machine();
        // No player_obj, no fallback → "(press i)" hint.
        let line = format_inventory_line(true, None, &[], &machine);
        assert_eq!(line, Some("Inv: (press i)".to_string()));

        // No player_obj but fallback available.
        let items = vec!["brass lamp".to_string()];
        let line2 = format_inventory_line(true, None, &items, &machine);
        assert!(line2.unwrap().contains("brass lamp"));

        // show_inventory = false → None.
        let line3 = format_inventory_line(false, None, &items, &machine);
        assert_eq!(line3, None);
    }

    // ── Task 5: status_msg render ─────────────────────────────────────────────

    #[test]
    fn status_msg_renders_on_status_line() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.status_msg = Some("saved".to_string());

        // 10-row area; status row is y=0.
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Row 0 (status line) must contain the word "saved".
        let status_row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            status_row.contains("saved"),
            "status row should contain 'saved' when status_msg is set; got: {:?}",
            status_row
        );
    }

    // ── Task 8: status-header + input-line boxing + opt-out ───────────────────

    /// Default (status_header_style = None): top row is the plain reversed bar
    /// with no border glyphs.  When status_header_style = Single, the status row
    /// is wrapped in a box (3 rows: top border, content, bottom border).
    #[test]
    fn status_header_plain_by_default_boxed_when_styled() {
        let machine = minimal_machine();

        // -- Default: plain reversed bar, no border glyphs --
        {
            let state = AppState::default();
            // status_header_style defaults to None
            assert!(
                matches!(state.colors.status_header_style, BorderStyle::None),
                "default status_header_style must be None"
            );

            let area = Rect::new(0, 0, 40, 10);
            let mut buf = Buffer::empty(area);
            render_transcript(&machine, &state, area, &mut buf);

            // Row 0 (status) must have REVERSED modifier (plain bar style).
            let top_cell = buf.cell((0, 0)).expect("top-left must exist");
            assert!(
                top_cell.modifier.contains(Modifier::REVERSED),
                "default status row must be reversed-video (plain bar)"
            );

            // No box corners in the top 3 rows.
            let has_corner_glyph = (0..3u16).any(|y| {
                (0..40u16).any(|x| {
                    let sym = buf.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                    matches!(sym, "┌" | "└" | "┐" | "┘" | "╔" | "╚" | "╗" | "╝" | "┏" | "┗" | "┓" | "┛")
                })
            });
            assert!(
                !has_corner_glyph,
                "default (None) status header must not render box corners"
            );
        }

        // -- Boxed: status_header_style = Single → 3-row box around status --
        {
            let mut state = AppState::default();
            state.colors.status_header_style = BorderStyle::Single;
            state.colors.status_header_sides = crate::render::paneframe::PaneSides::all(BorderStyle::Single);

            // Use a large enough area so boxing is not suppressed (needs >= 5 rows).
            let area = Rect::new(0, 0, 40, 12);
            let mut buf = Buffer::empty(area);
            render_transcript(&machine, &state, area, &mut buf);

            // Row 0 must be the top border: top-left corner must be "┌".
            assert_eq!(
                buf.cell((0, 0)).unwrap().symbol(),
                "┌",
                "boxed status header top-left must be a single-border corner"
            );
            // Row 1 (content) must have the status text with REVERSED style.
            // col 0 is the side border glyph; col 1 is the first content cell.
            let content_cell = buf.cell((1, 1)).expect("status content row must exist");
            assert!(
                content_cell.modifier.contains(Modifier::REVERSED),
                "status content row (inside box) must have REVERSED modifier"
            );
            // Row 2 must be the bottom border: bottom-left corner must be "└".
            assert_eq!(
                buf.cell((0, 2)).unwrap().symbol(),
                "└",
                "boxed status header bottom-left must be a single-border corner"
            );
        }
    }

    /// Default (input_line_style = None): bottom row is a plain `> ` prompt with
    /// no border glyphs.
    #[test]
    fn input_line_plain_by_default() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input = "go north".to_string();
        state.focus = Focus::Game;

        // input_line_style defaults to None
        assert!(
            matches!(state.colors.input_line_style, BorderStyle::None),
            "default input_line_style must be None"
        );

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);

        // Bottom row (y=9) must contain "> go north" (plain, no box).
        let bottom_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(
            bottom_row.contains("> go north"),
            "default input row must contain '> go north'; got: {:?}",
            bottom_row
        );

        // No corner glyphs in the bottom 3 rows.
        let has_corner = (7u16..10u16).any(|y| {
            (0..40u16).any(|x| {
                let sym = buf.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                matches!(sym, "┌" | "└" | "┐" | "┘" | "╔" | "╚" | "╗" | "╝" | "┏" | "┗" | "┓" | "┛")
            })
        });
        assert!(
            !has_corner,
            "default (None) input line must not render box corners"
        );
    }

    /// With map_border_style = None and story_border_style = None, calling draw_pane_frame
    /// with those styles produces no border glyphs (the opt-out path).  This is the
    /// "plain borderless" mode that reproduces the pre-beautification pane appearance.
    #[test]
    fn panes_none_reproduce_plain_borderless() {
        use crate::render::paneframe::{draw_pane_frame, BorderStyle};
        use ratatui::style::Style;

        // Resolve a color scheme with none borders (simulate `map_border = none`).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf_map = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf_map, area, BorderStyle::None, Style::default());

        // Content must equal the full area (no inset).
        assert_eq!(
            frame.content, area,
            "BorderStyle::None must return content == area (no inset)"
        );

        // No border glyphs anywhere in the buffer.
        let has_border_glyph = (0..10u16).any(|y| {
            (0..20u16).any(|x| {
                let sym = buf_map.cell((x, y)).map(|c| c.symbol()).unwrap_or("");
                matches!(sym,
                    "┌" | "─" | "┐" | "│" | "└" | "┘" |
                    "╔" | "═" | "╗" | "║" | "╚" | "╝" |
                    "┏" | "━" | "┓" | "┃" | "┗" | "┛"
                )
            })
        });
        assert!(
            !has_border_glyph,
            "BorderStyle::None must not render any border glyphs (opt-out path)"
        );

        // Same for story pane simulation.
        let mut buf_story = Buffer::empty(area);
        let story_frame = draw_pane_frame(&mut buf_story, area, BorderStyle::None, Style::default());
        assert_eq!(story_frame.content, area, "story pane None border must also have content == area");
    }

    #[test]
    fn status_header_left_right_only_draws_side_bars_no_top() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // base none, left/right single, large enough to box.
        state.colors.status_header_style = crate::render::paneframe::BorderStyle::None;
        state.colors.status_header_sides = crate::render::paneframe::PaneSides {
            top: crate::render::paneframe::BorderStyle::None,
            bottom: crate::render::paneframe::BorderStyle::None,
            left: crate::render::paneframe::BorderStyle::Single,
            right: crate::render::paneframe::BorderStyle::Single,
        };
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&machine, &state, area, &mut buf);
        // The left side bar must actually be drawn in column 0 of the boxed status
        // region (rows 0..3) — this is the headline left/right-only use case and
        // must NOT be inert. (Regression guard: with the old base-only boxing gate
        // nothing was boxed and this column was blank.)
        let has_left_bar = (0u16..3).any(|y| buf.cell((0, y)).map(|c| c.symbol()) == Some("│"));
        assert!(has_left_bar, "left/right-only status header must draw a side bar in column 0");
        // The right side bar too.
        let has_right_bar = (0u16..3).any(|y| buf.cell((39, y)).map(|c| c.symbol()) == Some("│"));
        assert!(has_right_bar, "left/right-only status header must draw a side bar at the right edge");
        // And no top corner glyph (top side is off).
        assert_ne!(buf.cell((0, 0)).unwrap().symbol(), "┌");
    }

    // ── draw_str_highlighted regression tests ────────────────────────────────

    /// Helper: render `draw_str_highlighted` into a fresh buffer and return the
    /// symbol string for row 0 (concatenated cell symbols).
    fn highlighted_row(text: &str, query_lower: &str, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            text,
            Style::default(),
            query_lower,
            Style::new().fg(ratatui::style::Color::Yellow),
            area,
        );
        (0..width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().to_string()).unwrap_or_default())
            .collect()
    }

    /// Turkish dotted-I (U+0130, 2 UTF-8 bytes) lowercases to the two-byte
    /// sequence "i\u{307}" (3 UTF-8 bytes).  When such a char precedes the
    /// search query, the old byte-offset arithmetic would produce a
    /// non-char-boundary panic.  This test verifies the fix: no panic and the
    /// correct glyphs appear in the buffer.
    #[test]
    fn draw_str_highlighted_dotted_i_no_panic() {
        // "İkey diary" with query "key": İ precedes the match, offsets shift.
        let row = highlighted_row("İkey diary", "key", 20);
        // Must contain the glyphs for the original text (no panic).
        assert!(row.contains('İ'), "dotted-I glyph must appear; got: {:?}", row);
        assert!(row.contains('k'), "k of key must appear; got: {:?}", row);
        assert!(row.contains('e'), "e of key must appear; got: {:?}", row);
        assert!(row.contains('y'), "y of key must appear; got: {:?}", row);
    }

    /// Verify the highlight STYLE is applied to the matched segment and only to
    /// it.  We use a plain ASCII line so the style boundaries are unambiguous.
    #[test]
    fn draw_str_highlighted_ascii_highlight_style() {
        let area = Rect::new(0, 0, 20, 1);
        let mut buf = Buffer::empty(area);
        let highlight_style = Style::new().fg(ratatui::style::Color::Yellow);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "hello world",
            Style::default(),
            "world",
            highlight_style,
            area,
        );

        // "hello " (6 chars, x=0..5) must NOT have Yellow fg.
        for x in 0u16..6 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_ne!(
                cell.style().fg,
                Some(ratatui::style::Color::Yellow),
                "x={} should not be highlighted", x
            );
        }
        // "world" (5 chars, x=6..10) must have Yellow fg.
        for x in 6u16..11 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_eq!(
                cell.style().fg,
                Some(ratatui::style::Color::Yellow),
                "x={} should be highlighted", x
            );
        }
    }

    /// Multiple occurrences of the query on the same line all get highlighted.
    #[test]
    fn draw_str_highlighted_multiple_matches() {
        let area = Rect::new(0, 0, 30, 1);
        let mut buf = Buffer::empty(area);
        let highlight_style = Style::new().fg(ratatui::style::Color::Yellow);
        // "aXa" with query "a": matches at x=0 and x=2.
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "aXa",
            Style::default(),
            "a",
            highlight_style,
            area,
        );
        let cell0 = buf.cell((0, 0)).unwrap();
        let cell2 = buf.cell((2, 0)).unwrap();
        assert_eq!(cell0.style().fg, Some(ratatui::style::Color::Yellow), "first 'a' highlighted");
        assert_eq!(cell2.style().fg, Some(ratatui::style::Color::Yellow), "second 'a' highlighted");
        let cell1 = buf.cell((1, 0)).unwrap();
        assert_ne!(cell1.style().fg, Some(ratatui::style::Color::Yellow), "'X' not highlighted");
    }

    /// Empty query draws text with base style and no panic.
    #[test]
    fn draw_str_highlighted_empty_query_no_highlight() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        draw_str_highlighted(
            &mut buf,
            0, 0,
            "hello",
            Style::default(),
            "",
            Style::new().fg(ratatui::style::Color::Yellow),
            area,
        );
        for x in 0u16..5 {
            let cell = buf.cell((x, 0)).unwrap();
            assert_ne!(cell.style().fg, Some(ratatui::style::Color::Yellow), "no highlight for empty query at x={}", x);
        }
    }

    /// Query longer than text produces no match and no panic.
    #[test]
    fn draw_str_highlighted_query_longer_than_text_no_panic() {
        let row = highlighted_row("hi", "hello world", 10);
        assert!(row.contains('h'), "text glyphs must still appear; got: {:?}", row);
    }
}
