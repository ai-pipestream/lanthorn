//! GAME pane rendering: status line (top), scrolling transcript (middle), input line (bottom).
//!
//! The inventory is shown in a separate docked panel below the input line
//! (see `render::inventory_dock`), not inside this pane.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use ratatui::style::Color;

use crate::engine::{Introspect, StatusField, StatusModel};
use crate::state::{AppState, Focus, ParaFmt, StyleRun, TranscriptFilter, TranscriptKind};
use crate::render::paneframe::{draw_framed, BorderStyle};
use super::draw_str_clipped;

/// One wrapped display row: its `text`, `kind`, resolved base `style`, per-run
/// style `runs`, and — for a row that is part of an inline-image band — the
/// `band` geometry to blit (Task 8). Text rows carry `band: None`.
#[derive(Clone)]
pub(crate) struct WrappedRow {
    pub text: String,
    pub kind: TranscriptKind,
    pub style: Style,
    pub runs: Vec<StyleRun>,
    /// For a row that is part of an inline-image band, the geometry to blit
    /// (Task 8). Text rows carry `None`.
    pub band: Option<ImageBand>,
    /// For a row that flows *beside* a left-margin float (SQ-0454): the image
    /// strip to blit at the left margin (`x_off == 0`) after the row's text is
    /// drawn. Unlike `band`, a float row also carries text (already indented past
    /// the picture); a leftover float row taller than its text carries an empty
    /// `text`. `None` for ordinary rows and for band rows.
    pub float: Option<ImageBand>,
}

/// Geometry for one terminal row of an inline-image band: the source `image`,
/// the band's total `cols`x`rows` cell footprint, this row's index `row` in
/// `0..rows`, and the horizontal cell offset `x_off` (nonzero for margin-right).
///
/// The fields are read by the Task 8 blitter (`render/inline_image.rs`); bands
/// are only constructed under `images_enabled`.
#[derive(Clone)]
pub(crate) struct ImageBand {
    pub image: crate::inline_image::InlineImage,
    pub cols: u16,
    pub rows: u16,
    pub row: u16,
    pub x_off: u16,
}

// ── Styles ─────────────────────────────────────────────────────────────────────
//
// Status, normal text, and suggestion styles are read from `state.colors` at
// render time.  The CURSOR style remains a local constant as it is structural
// (REVERSED only, no color content mapped by ColorScheme).

pub const CURSOR_STYLE: Style = Style::new()
    .add_modifier(Modifier::REVERSED);

/// The block-cursor style for the input caret. A cursor is reverse-video of the
/// text it sits on, so when the game has set page colours (`game_input` is
/// `Some`) reverse the resolved input `text_style` — otherwise a bare REVERSED
/// cursor reverses the *theme*, which can render near-invisible on a recoloured
/// page (e.g. a white game background). With no game colour it stays the
/// structural theme cursor. (SQ-0268)
pub(crate) fn cursor_style(text_style: Style, game_input: Option<Style>) -> Style {
    match game_input {
        Some(_) => text_style.add_modifier(Modifier::REVERSED),
        None => CURSOR_STYLE,
    }
}

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
    // This is where the {location} segment sits, so an overlong name breaks at a
    // space and gets an ellipsis (ZMSD §8.2.2.2) instead of a hard clip.
    let left_budget = right_start;
    {
        let mut x = 0usize;
        for (t, s, _) in &left {
            if x >= left_budget { break; }
            let avail = left_budget - x;
            let txt = truncate_status_text(t, avail);
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

/// Truncate a status-bar segment to `width` columns the way ZMSD §8.2.2.2 asks:
/// "If the object's short name exceeds the available room on the status line,
/// the author suggests that an interpreter should break it at the last space and
/// append an ellipsis". We use the single-character ellipsis '…' rather than the
/// spec's three dots so the marker itself costs one column, not three.
///
/// Only applied when the text actually overflows — a segment that fits is
/// returned unchanged, so nothing gains a spurious '…'. A single word longer
/// than `width` (no space to break at) falls back to a hard character break,
/// still marked with the ellipsis.
pub(crate) fn truncate_status_text(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let head: String = text.chars().take(width - 1).collect(); // one column for '…'
    let kept = match head.rfind(' ') {
        Some(i) => head[..i].trim_end(),
        None => head.as_str(),
    };
    format!("{kept}…")
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
    wrap_line_ranges(line, width).into_iter().map(|(s, _, _)| s).collect()
}

/// Like `wrap_line`, but each emitted row carries the `[start, end)` char range
/// it occupies in the *original* `line` (so per-line style runs can be re-based
/// onto wrapped rows). The break space dropped between two rows is excluded from
/// both: row N ends before the space, row N+1 starts after it. Hard-broken long
/// words split into contiguous ranges.
pub(crate) fn wrap_line_ranges(line: &str, width: u16) -> Vec<(String, usize, usize)> {
    wrap_line_ranges_nw(line, width, None)
}

/// [`wrap_line_ranges`] with the Z-machine's `buffer_mode` honoured (ZMSD §7.2.1):
/// `nowrap_from` is the char offset at/after which the game had buffering OFF, so
/// from there on the text must break after the last character that FITS — no
/// word-wrap, no word carried to the next row (a long word splits mid-word, and
/// the break space is kept rather than swallowed).
///
/// Rule for a MIXED line (buffering turned off part-way through): a row
/// word-wraps only while it lies entirely inside the buffered prefix; the first
/// row that reaches into the unbuffered region — and every row after it —
/// char-breaks. This matches the spec's model (buffering is a property of the
/// moment the text was printed) without needing per-word state.
fn wrap_line_ranges_nw(line: &str, width: u16, nowrap_from: Option<usize>) -> Vec<(String, usize, usize)> {
    let width = width as usize;
    if width == 0 {
        return vec![(line.to_string(), 0, line.chars().count())];
    }
    if line.is_empty() {
        return vec![(String::new(), 0, 0)];
    }

    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut remaining = line;
    let mut base_col = 0usize; // char offset of `remaining`'s start within `line`

    while !remaining.is_empty() {
        let char_count: usize = remaining.chars().count();
        if char_count <= width {
            rows.push((remaining.to_string(), base_col, base_col + char_count));
            break;
        }

        // This row reaches into text printed with buffering off → hard char-break.
        let unbuffered = nowrap_from.is_some_and(|n| n < base_col + width);

        // Collect byte offsets for chars 0..=width (inclusive so we see the boundary char).
        // We want the last space at char index ≤ width to break at.
        let mut last_space_before: Option<usize> = None; // byte offset of last space in 0..width
        let mut byte_at_width: usize = remaining.len();  // byte offset of char #width (the first that doesn't fit)
        for (i, (byte_i, ch)) in remaining.char_indices().enumerate() {
            if i == width {
                byte_at_width = byte_i;
                // If the char right at the boundary is a space, break here
                // (the row is exactly `width` non-space chars).
                if ch == ' ' && !unbuffered {
                    last_space_before = Some(byte_i);
                }
                break;
            }
            if ch == ' ' && !unbuffered {
                last_space_before = Some(byte_i);
            }
        }

        if let Some(sp) = last_space_before {
            // Break at the space: take everything before it, skip the space.
            let head = &remaining[..sp];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            // Advance past the space (sp is a byte offset of ' ', so sp+1 is safe for ASCII ' ').
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
            base_col += head_chars + 1; // +1 for the dropped break space
        } else {
            // No space found: hard-break at `width` chars.
            let head = &remaining[..byte_at_width];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            remaining = &remaining[byte_at_width..];
            base_col += head_chars;
        }
    }

    if rows.is_empty() {
        rows.push((String::new(), 0, 0));
    }
    rows
}

/// Intersect a logical line's `StyleRun`s with a wrapped row's `[start, end)`
/// source char range, re-basing the surviving spans to the row's own offsets.
/// Empty intersections are dropped (so an unstyled row yields an empty vec).
fn rebase_runs(line_runs: Option<&Vec<StyleRun>>, start: usize, end: usize) -> Vec<StyleRun> {
    let Some(line_runs) = line_runs else {
        return Vec::new();
    };
    let mut out: Vec<StyleRun> = Vec::new();
    for r in line_runs {
        let s = r.start.max(start);
        let e = r.end.min(end);
        if s < e {
            out.push(StyleRun { start: s - start, end: e - start, bits: r.bits, fg: r.fg, bg: r.bg, link: r.link, glk_style: r.glk_style });
        }
    }
    out
}

/// Shift every run in `runs` right by `pad` columns (added leading spaces), so a
/// row's style runs stay aligned with its padded/justified text (SQ-0330).
fn shift_runs(runs: Vec<StyleRun>, pad: u16) -> Vec<StyleRun> {
    if pad == 0 {
        return runs;
    }
    let p = pad as usize;
    runs.into_iter()
        .map(|mut r| {
            r.start += p;
            r.end += p;
            r
        })
        .collect()
}

/// Wrap a Story/Input logical `line` into display rows honouring its Glk paragraph
/// layout `pf` (SQ-0330). Returns one `(text, start, end, pad)` per wrapped row:
/// `text` is the padded row (leading spaces already prepended), `[start, end)` are
/// the row's char offsets in the ORIGINAL `line` (for run re-basing), and `pad` is
/// the number of leading spaces added (so callers can shift the row's runs).
///
/// Layout is rendered purely as LEADING-SPACE padding so the drawn text, its runs,
/// and selection/search coordinates stay consistent:
/// - `indent` cells indent every row; `para_indent` adds to the FIRST row only
///   (negative = hanging first line), clamped so a row still fits.
/// - Justification pads within the row's usable width: Centered → half the slack,
///   RightFlush → all of it, LeftFlush/LeftRight (fill) → none (fill is treated as
///   left for now; full inter-word fill is out of scope).
///
/// A default `pf` (left, no indent) reduces to `wrap_line_ranges` with `pad == 0`,
/// so the Z-machine path and un-hinted buffers render byte-identically.
///
/// `pf.nowrap_from` (the game switched `buffer_mode` off) makes the affected rows
/// char-break instead of word-wrap — see [`wrap_line_ranges_nw`].
fn wrap_para_ranges(line: &str, width: u16, pf: ParaFmt) -> Vec<(String, usize, usize, u16)> {
    let nw = pf.nowrap_from.map(|n| n as usize);
    // Fast path: the common no-layout case is exactly the old behaviour (an
    // unbuffered Z-machine line carries no Glk layout, so it lands here too).
    let no_layout = pf.indent == 0 && pf.para_indent == 0 && pf.justify == 0;
    if no_layout || width == 0 {
        return wrap_line_ranges_nw(line, width, nw)
            .into_iter()
            .map(|(t, s, e)| (t, s, e, 0))
            .collect();
    }
    let wmax = width.saturating_sub(1); // always leave room for at least 1 char
    let indent = pf.indent.min(wmax);
    // Row-0 indent = indent + para_indent, clamped into [0, wmax].
    let row0_indent = (indent as i32 + pf.para_indent as i32).clamp(0, wmax as i32) as u16;
    let cont_w = width.saturating_sub(indent).max(1);
    let first_w = width.saturating_sub(row0_indent).max(1);

    // Wrap row 0 at first_w, then the remainder at cont_w, keeping original char
    // offsets so runs re-base correctly.
    let mut ranges: Vec<(String, usize, usize)> = wrap_line_ranges_nw(line, first_w, nw);
    if ranges.len() > 1 && cont_w != first_w {
        let second_start = ranges[1].1;
        let remainder: String = line.chars().skip(second_start).collect();
        ranges.truncate(1);
        // The remainder is re-wrapped in its own coordinates, so rebase `nw` too.
        let rem_nw = nw.map(|n| n.saturating_sub(second_start));
        for (t, s, e) in wrap_line_ranges_nw(&remainder, cont_w, rem_nw) {
            ranges.push((t, s + second_start, e + second_start));
        }
    }

    ranges
        .into_iter()
        .enumerate()
        .map(|(ri, (text, s, e))| {
            let lead = if ri == 0 { row0_indent } else { indent };
            let usable = if ri == 0 { first_w } else { cont_w };
            let rowlen = text.chars().count() as u16;
            let slack = usable.saturating_sub(rowlen);
            let just_pad = match pf.justify {
                2 => slack / 2,   // Centered
                3 => slack,       // RightFlush
                _ => 0,           // LeftFlush / LeftRight (fill → left for now)
            };
            let pad = lead + just_pad;
            let padded = if pad == 0 { text } else { format!("{}{}", " ".repeat(pad as usize), text) };
            (padded, s, e, pad)
        })
        .collect()
}

/// Like `wrap_line`, but every continuation row after the first is prefixed
/// with `indent` spaces so wrapped text hangs under the first row's content.
pub(crate) fn wrap_line_hanging(line: &str, width: u16, indent: u16) -> Vec<String> {
    let indent = (indent as usize).min(width.saturating_sub(1) as usize);
    if width == 0 || (line.chars().count() as u16) <= width {
        return wrap_line(line, width);
    }
    // Wrap the body at the reduced width, then re-prefix continuations.
    let first = wrap_line(line, width);
    let mut out: Vec<String> = Vec::new();
    for (i, row) in first.into_iter().enumerate() {
        if i == 0 {
            out.push(row);
        } else {
            // Re-wrap continuation content within (width - indent) to keep the
            // hang stable, prefixing the indent.
            let pad = " ".repeat(indent);
            for sub in wrap_line(&row, width.saturating_sub(indent as u16)) {
                out.push(format!("{pad}{sub}"));
            }
        }
    }
    out
}

/// Count leading ASCII spaces in `s`.
pub(crate) fn leading_spaces(s: &str) -> u16 {
    s.chars().take_while(|c| *c == ' ').count() as u16
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
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
) -> Vec<WrappedRow> {
    let mut out: Vec<WrappedRow> = Vec::new();
    // The currently-active left-margin float (Zork Zero's drop-cap idiom): its
    // picture occupies the left `indent` columns and the following Story/Input
    // rows wrap beside it. Only ONE float is active at a time; a new image or a
    // non-prose line flushes it first (so the whole picture always renders, even
    // when the text beside it is shorter than the image — or absent). (SQ-0454)
    let mut float: Option<FloatState> = None;

    for (i, line) in transcript.iter().enumerate() {
        // An image unit either starts a left-margin float or expands into an
        // N-row band (or zero rows when images are disabled). `images.get(i)`
        // yields `None` for a plain text line (and for a short `images` slice).
        if let Some(Some(img)) = images.get(i) {
            if !images_enabled {
                continue;
            }
            // A new picture ends any active float (finishing it as strip rows).
            flush_float(&mut out, &mut float);
            if left_float {
                if let Some(fl) = FloatState::start(img, char_px, width) {
                    float = Some(fl);
                    continue; // the float emits no rows of its own; text rides beside it
                }
            }
            // Band fallback: inline aligns, margin-right, floats-disabled, or a
            // left-margin image too wide (or too cramped) to float.
            let (cols, rows) = img.fitted_cells(width, char_px);
            let x_off = match img.align {
                crate::inline_image::ImageAlign::MarginRight => width.saturating_sub(cols),
                _ => 0,
            };
            for r in 0..rows {
                out.push(WrappedRow {
                    text: String::new(),
                    kind: TranscriptKind::Story,
                    style: Style::default(),
                    runs: Vec::new(),
                    band: Some(ImageBand { image: img.clone(), cols, rows, row: r, x_off }),
                    float: None,
                });
            }
            continue;
        }

        let kind = kinds.get(i).copied().unwrap_or(TranscriptKind::Story);
        let style = styles.get(i).copied().unwrap_or_default();
        let line_runs = runs.get(i);
        let is_prose = matches!(kind, TranscriptKind::Story | TranscriptKind::Input);
        // A float only wraps the game's own prose. Any other line kind flushes
        // the picture first, then renders normally at full width.
        if !is_prose {
            flush_float(&mut out, &mut float);
        }

        match kind {
            // Meta/Warning are app-generated (always unstyled) and use hanging
            // wrap, whose indentation shifts offsets — emit empty runs.
            TranscriptKind::Meta | TranscriptKind::Warning => {
                let w = width.saturating_sub(META_GUTTER);
                for row in wrap_line_hanging(line, w, leading_spaces(line).max(2)) {
                    out.push(WrappedRow { text: row, kind, style, runs: Vec::new(), band: None, float: None });
                }
            }
            TranscriptKind::Story | TranscriptKind::Input if float.is_some() => {
                // Wrap this prose line beside the active float: the first `rem`
                // output rows are narrowed by the float's `reserve` and carry the
                // next image strip; rows past the picture reclaim full width. The
                // rows are padded by `pad` (nonzero for a LEFT float, pushing text
                // right past the picture; zero for a RIGHT float, flush left) and
                // the strip blits at the float's `x_off`.
                // Copy the geometry up front so the row loop borrows nothing.
                let (image, cols, total, reserve, pad, x_off, start_strip, rem) = {
                    let fl = float.as_ref().unwrap();
                    (fl.image.clone(), fl.cols, fl.rows, fl.reserve, fl.pad, fl.x_off, fl.next_strip, fl.remaining() as usize)
                };
                let narrow = width.saturating_sub(reserve).max(1);
                let nw = para.get(i).and_then(|p| p.nowrap_from).map(|n| n as usize);
                let ranges = wrap_line_ranges_var(line, nw, |k| if k < rem { narrow } else { width });
                let nrows = ranges.len();
                for (k, (text, start, end)) in ranges.into_iter().enumerate() {
                    let (pad, float_band) = if k < rem {
                        let band = ImageBand { image: image.clone(), cols, rows: total, row: start_strip + k as u16, x_off };
                        (pad, Some(band))
                    } else {
                        (0, None)
                    };
                    let padded = if pad == 0 { text } else { format!("{}{}", " ".repeat(pad as usize), text) };
                    out.push(WrappedRow {
                        text: padded,
                        kind,
                        style,
                        // Shift the row's runs right by the leading padding so
                        // selection/copy/search coordinates match the drawn text.
                        runs: shift_runs(rebase_runs(line_runs, start, end), pad),
                        band: None,
                        float: float_band,
                    });
                }
                // Advance the float by the strips just placed; retire it once the
                // whole picture has been laid down.
                let placed = nrows.min(rem) as u16;
                let fl = float.as_mut().unwrap();
                fl.next_strip += placed;
                if fl.remaining() == 0 {
                    float = None;
                }
            }
            TranscriptKind::Story | TranscriptKind::Input => {
                let pf = para.get(i).copied().unwrap_or_default();
                for (row, start, end, pad) in wrap_para_ranges(line, width, pf) {
                    out.push(WrappedRow {
                        text: row,
                        kind,
                        style,
                        // Shift the row's runs right by the leading padding so
                        // selection/copy/search coordinates match the padded
                        // text that is actually drawn (SQ-0330).
                        runs: shift_runs(rebase_runs(line_runs, start, end), pad),
                        band: None,
                        float: None,
                    });
                }
            }
        }
    }
    // Finish any float whose picture outran (or had no) text beside it.
    flush_float(&mut out, &mut float);
    out
}

/// The minimum prose column (cells) worth floating a picture beside; a narrower
/// column falls back to a full-width band.
const FLOAT_MIN_TEXT_COLS: u16 = 8;

/// An in-progress margin float while `wrap_lines_kinded` walks the transcript:
/// the source `image`, its cell footprint (`cols` wide × `rows` tall), and how
/// its rows lay out. The float side is expressed by three geometry fields rather
/// than an enum:
/// - LEFT float (Zork Zero's drop-cap): `pad == reserve` pushes text right past
///   the picture, `x_off == 0` blits it at the left.
/// - RIGHT float (Shogun's opening picture, ZMSD §15 margin picture): `pad == 0`
///   keeps text flush left, `x_off` blits the picture at the right edge.
///
/// Either way the wrap width on covered rows is `width - reserve`. (SQ-0454/0489)
struct FloatState {
    image: crate::inline_image::InlineImage,
    cols: u16,
    rows: u16,
    /// Columns removed from the text width on covered rows.
    reserve: u16,
    /// Leading pad added to covered rows' text (left float pushes text right).
    pad: u16,
    /// Image band x offset (right float right-aligns the picture).
    x_off: u16,
    next_strip: u16,
}

impl FloatState {
    /// Begin a float for a `MarginLeft` or `MarginRight` image, or `None` to fall
    /// back to a band: any other align, or a picture that leaves no prose column.
    fn start(img: &crate::inline_image::InlineImage, char_px: (u16, u16), width: u16) -> Option<FloatState> {
        let (cols, rows) = img.fitted_cells(width, char_px);
        if cols == 0 || rows == 0 {
            return None;
        }
        match img.align {
            crate::inline_image::ImageAlign::MarginLeft => {
                // Starve-guard: a left drop-cap wider than ~half the viewport
                // falls back to a band (the historic SQ-0454 rule).
                if cols.saturating_mul(2) > width {
                    return None;
                }
                let indent = float_text_indent(img, char_px.0, cols);
                if width.saturating_sub(indent) < FLOAT_MIN_TEXT_COLS {
                    return None; // no room for text beside the picture
                }
                Some(FloatState { image: img.clone(), cols, rows, reserve: indent, pad: indent, x_off: 0, next_strip: 0 })
            }
            crate::inline_image::ImageAlign::MarginRight => {
                // Reserve the picture's own cell width plus a one-column gutter;
                // the picture right-aligns and text stays flush left. Fall back to
                // a band when that leaves no usable prose column.
                let reserve = (cols + 1).min(width);
                if width.saturating_sub(reserve) < FLOAT_MIN_TEXT_COLS {
                    return None;
                }
                Some(FloatState { image: img.clone(), cols, rows, reserve, pad: 0, x_off: width.saturating_sub(cols), next_strip: 0 })
            }
            _ => None,
        }
    }

    /// Strip rows not yet placed.
    fn remaining(&self) -> u16 {
        self.rows.saturating_sub(self.next_strip)
    }

    /// The band geometry for strip `row` of this float, blitted at its `x_off`.
    fn strip(&self, row: u16) -> ImageBand {
        ImageBand { image: self.image.clone(), cols: self.cols, rows: self.rows, row, x_off: self.x_off }
    }
}

/// The text indent (in cells) for a left-margin float: the game's own
/// `set_margins` value (`margin_px`, in GAME pixels — scaled the same way the
/// picture is, then rounded up to whole cells) when present, else the picture's
/// cell width plus a one-column gutter. Never less than the picture width, so
/// prose can't overlap the image. (SQ-0454)
fn float_text_indent(img: &crate::inline_image::InlineImage, cell_w: u16, cols: u16) -> u16 {
    let cell_w = cell_w.max(1) as u32;
    let indent = match img.margin_px {
        Some(margin) => {
            // `scaled` already encodes `v6_image_scale`; derive the same factor
            // and apply it to the game-pixel margin so text lines up with the
            // scaled picture.
            let native_w = img.pixels.width().max(1);
            let scaled_w = img.scaled.map(|(w, _)| w).unwrap_or(native_w).max(1);
            let scaled_margin = (margin as u64 * scaled_w as u64 / native_w as u64) as u32;
            scaled_margin.div_ceil(cell_w).max(1) as u16
        }
        None => cols + 1,
    };
    indent.max(cols)
}

/// Emit a float's not-yet-placed strips as empty rows so its whole picture
/// renders even when the text beside it is shorter than the image (or absent),
/// then clear the float. (SQ-0454)
fn flush_float(out: &mut Vec<WrappedRow>, float: &mut Option<FloatState>) {
    if let Some(fl) = float.take() {
        for r in fl.next_strip..fl.rows {
            out.push(WrappedRow {
                text: String::new(),
                kind: TranscriptKind::Story,
                style: Style::default(),
                runs: Vec::new(),
                band: None,
                float: Some(fl.strip(r)),
            });
        }
    }
}

/// Word-wrap `line` into rows where output row `k` may use a different width
/// `width_for(k)` (clamped to ≥ 1). Like [`wrap_line_ranges`] but the usable
/// width can shrink or grow per row — used to wrap prose beside a left-margin
/// float (narrow while the picture is present, full width once it ends). Each
/// row carries its `[start, end)` char range in the original `line` so per-line
/// style runs can be re-based onto the wrapped rows. (SQ-0454)
///
/// `nowrap_from` char-breaks the rows that reach into unbuffered text, exactly as
/// in [`wrap_line_ranges_nw`] (ZMSD §7.2.1).
fn wrap_line_ranges_var(line: &str, nowrap_from: Option<usize>, width_for: impl Fn(usize) -> u16) -> Vec<(String, usize, usize)> {
    if line.is_empty() {
        return vec![(String::new(), 0, 0)];
    }
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut remaining = line;
    let mut base_col = 0usize; // char offset of `remaining`'s start within `line`

    while !remaining.is_empty() {
        let width = width_for(rows.len()).max(1) as usize;
        let char_count = remaining.chars().count();
        if char_count <= width {
            rows.push((remaining.to_string(), base_col, base_col + char_count));
            break;
        }
        // Last space at char index ≤ width to break at (see `wrap_line_ranges`);
        // suppressed once the row reaches unbuffered text (hard char-break).
        let unbuffered = nowrap_from.is_some_and(|n| n < base_col + width);
        let mut last_space_before: Option<usize> = None;
        let mut byte_at_width = remaining.len();
        for (idx, (byte_i, ch)) in remaining.char_indices().enumerate() {
            if idx == width {
                byte_at_width = byte_i;
                if ch == ' ' && !unbuffered {
                    last_space_before = Some(byte_i);
                }
                break;
            }
            if ch == ' ' && !unbuffered {
                last_space_before = Some(byte_i);
            }
        }
        if let Some(sp) = last_space_before {
            let head = &remaining[..sp];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            let next = sp + ' '.len_utf8();
            remaining = &remaining[next..];
            base_col += head_chars + 1; // +1 for the dropped break space
        } else {
            let head = &remaining[..byte_at_width];
            let head_chars = head.chars().count();
            rows.push((head.to_string(), base_col, base_col + head_chars));
            remaining = &remaining[byte_at_width..];
            base_col += head_chars;
        }
    }
    if rows.is_empty() {
        rows.push((String::new(), 0, 0));
    }
    rows
}

/// Wrapped-row count before the screen-clear anchor `clear_anchor` (a source-line
/// index into the given slices), or `None` when the anchor is unset or out of
/// range. This is the number of display rows the pre-clear lines `[..a]` occupy,
/// used to top-anchor post-clear content. Defensively clamps each parallel slice
/// (`runs` and, in principle, the others, may be shorter than `transcript`).
/// (SQ-0305)
pub(crate) fn anchor_wrapped_rows(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    left_float: bool,
    width: u16,
    clear_anchor: Option<usize>,
) -> Option<usize> {
    let a = clear_anchor?;
    if a >= transcript.len() {
        return None;
    }
    Some(
        wrap_lines_kinded(
            &transcript[..a],
            &kinds[..a.min(kinds.len())],
            &styles[..a.min(styles.len())],
            &runs[..a.min(runs.len())],
            &para[..a.min(para.len())],
            &images[..a.min(images.len())],
            char_px,
            images_enabled,
            left_float,
            width,
        )
        .len(),
    )
}

/// Window the fully wrapped `display_rows` down to the `rows` rows visible at
/// `scroll` (0 = newest at bottom; higher = further back). `anchor_row` is the
/// pre-computed [`anchor_wrapped_rows`] value; when present and the view is at the
/// bottom (`scroll == 0`) and the post-clear content still fits, those lines are
/// pinned to the TOP (returning fewer than `rows` rows → the caller leaves the
/// rest blank) instead of bottom-sticking, which would pull pre-clear history
/// back into view. Older lines above the anchor stay reachable by scrolling up;
/// once post-clear content overflows the viewport this no longer triggers.
///
/// Returns (visible rows oldest-first, total wrapped-row count, first visible
/// absolute row). This does NOT wrap — the wrapping is done once by the caller
/// (cached across frames), so windowing/scroll is cheap. (SQ-0305)
pub(crate) fn window_wrapped_rows(
    display_rows: &[WrappedRow],
    anchor_row: Option<usize>,
    rows: usize,
    scroll: u16,
) -> (Vec<WrappedRow>, usize, usize) {
    if rows == 0 || display_rows.is_empty() {
        return (Vec::new(), 0, 0);
    }
    let n = display_rows.len();
    if scroll == 0 {
        if let Some(anchor_row) = anchor_row {
            if n.saturating_sub(anchor_row) <= rows {
                return (display_rows[anchor_row..n].to_vec(), n, anchor_row);
            }
        }
    }
    // Clamp scroll so it never exceeds the top: past `n - rows` the window would
    // otherwise shrink from the bottom, blanking viewport rows.
    let max_scroll = n.saturating_sub(rows);
    let scroll = (scroll as usize).min(max_scroll);
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(rows);
    (display_rows[start..end].to_vec(), n, start)
}

/// Return the **wrapped** display rows (with kinds) visible in `rows` rows,
/// honouring `scroll` (0 = newest at bottom; higher = further back in history).
///
/// The returned vec is ordered oldest-first so the caller can draw top-to-bottom.
/// Returns the visible window of wrapped rows AND the total wrapped-row count
/// (so callers can size a scrollbar without re-wrapping).
///
/// This wraps the whole slice every call; the main transcript pane instead caches
/// the wrapped product (see `AppState::transcript_wrap`) and calls
/// [`window_wrapped_rows`] directly. This entry point is retained for secondary
/// buffer windows and unit tests.
pub(crate) fn visible_wrapped_lines_kinded(
    transcript: &[String],
    kinds: &[TranscriptKind],
    styles: &[Style],
    runs: &[Vec<StyleRun>],
    para: &[ParaFmt],
    images: &[Option<crate::inline_image::InlineImage>],
    char_px: (u16, u16),
    images_enabled: bool,
    rows: usize,
    scroll: u16,
    width: u16,
    clear_anchor: Option<usize>,
) -> (Vec<WrappedRow>, usize, usize) {
    if rows == 0 || transcript.is_empty() {
        return (Vec::new(), 0, 0);
    }
    // Secondary buffer windows (the only caller besides tests) keep the legacy
    // band rendering for margin images — left-margin floats are a main-transcript
    // affordance (SQ-0454), so `left_float` is off here.
    let display_rows = wrap_lines_kinded(transcript, kinds, styles, runs, para, images, char_px, images_enabled, false, width);
    // Only the bottom (scroll == 0) view can top-anchor, so skip the extra wrap
    // of `[..a]` while scrolled back.
    let anchor_row = if scroll == 0 {
        anchor_wrapped_rows(transcript, kinds, styles, runs, para, images, char_px, images_enabled, false, width, clear_anchor)
    } else {
        None
    };
    window_wrapped_rows(&display_rows, anchor_row, rows, scroll)
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
///
/// Retained as the reference renderer for the search-highlight path: production
/// drawing now goes through `draw_str_runs` (which `highlight_mask` keeps
/// consistent with this function), and the tests assert the two stay identical.
#[cfg(test)]
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

/// Mark which char positions in `text` fall inside a case-insensitive match of
/// `query_lower`. Mirrors `draw_str_highlighted`'s lowered-string matching so the
/// two stay consistent. Returns one bool per char in `text`.
fn highlight_mask(text: &str, query_lower: &str) -> Vec<bool> {
    let nchars = text.chars().count();
    let mut mask = vec![false; nchars];
    if query_lower.is_empty() {
        return mask;
    }
    // Build a lowered copy and record each source char's lowered byte-start.
    let mut tl = String::with_capacity(text.len() + 4);
    let mut char_lb: Vec<usize> = Vec::with_capacity(nchars + 1);
    for ch in text.chars() {
        char_lb.push(tl.len());
        for lc in ch.to_lowercase() {
            tl.push(lc);
        }
    }
    char_lb.push(tl.len()); // sentinel

    let qlen = query_lower.len();
    let mut from = 0usize;
    while from < tl.len() {
        if let Some(rel) = tl[from..].find(query_lower) {
            let ms = from + rel;
            let me = ms + qlen;
            // Mark every source char whose lowered span overlaps [ms, me).
            for i in 0..nchars {
                if char_lb[i] < me && ms < char_lb[i + 1] {
                    mask[i] = true;
                }
            }
            from = me;
        } else {
            break;
        }
    }
    mask
}

/// Draw `text` at `(x, y)` applying per-char style: `base_style` plus the bits of
/// the `StyleRun` covering that char, and its resolved fg/bg colours. When
/// `search` is `Some((query_lower, highlight_style))`, characters inside a query
/// match use `highlight_style` instead (the search affordance wins over game
/// styling). With empty `runs` and no search this is byte-identical to
/// `draw_str_clipped`; with empty `runs` and a search it matches
/// `draw_str_highlighted`.
///
/// `scheme` is always supplied (for palette + per-Glk-style theme slots); `honor`
/// gates the GAME's own run colours (garglk `stylehint 0/1`): when off, a run's
/// game-set fg/bg is IGNORED, but the theme slot and element base still apply
/// (SQ-0331). Style bits (bold/italic/reverse) and the hyperlink affordance are
/// unaffected by `honor`, exactly as before.
pub(crate) fn draw_str_runs(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    text: &str,
    base_style: Style,
    runs: &[StyleRun],
    search: Option<(&str, Style)>,
    area: ratatui::layout::Rect,
    scheme: &crate::colors::ColorScheme,
    honor: bool,
) {
    if y < area.y || y >= area.bottom() {
        return;
    }
    let hi: Vec<bool> = match search {
        Some((q, _)) if !q.is_empty() => highlight_mask(text, q),
        _ => Vec::new(),
    };
    for (col, (i, ch)) in (x..).zip(text.chars().enumerate()) {
        if col >= area.right() {
            break;
        }
        let style = if hi.get(i).copied().unwrap_or(false) {
            search.unwrap().1
        } else {
            let run = runs.iter().find(|r| i >= r.start && i < r.end);
            let bits = run.map(|r| r.bits).unwrap_or(0);
            let mut s = crate::render::apply_text_style(base_style, bits);
            // Per-channel colour resolution (SQ-0331): game-set run colour (gated
            // by `honor`), then the theme's per-Glk-style slot (buffer = row 0),
            // then the element base (`base_style`). fg/bg are logical (pre-reverse);
            // apply_text_style already set REVERSED for bit 1, so the terminal
            // performs exactly one swap.
            {
                use crate::state::unpack_zcolour;
                use zvm::screen::ZColour;
                use crate::render::resolve_glk_channel;
                let game = |packed: u32| -> Option<ratatui::style::Color> {
                    let z = unpack_zcolour(packed);
                    (!matches!(z, ZColour::Default)).then(|| crate::render::resolve_zcolour(z, scheme))
                };
                let glk = run.map(|r| r.glk_style as usize).unwrap_or(0);
                let slot = scheme.glk_styles[0].get(glk).copied().unwrap_or_default();
                let game_fg = run.and_then(|r| game(r.fg));
                let game_bg = run.and_then(|r| game(r.bg));
                if let Some(c) = resolve_glk_channel(game_fg, slot.fg, base_style.fg, honor) {
                    s = s.fg(c);
                }
                if let Some(c) = resolve_glk_channel(game_bg, slot.bg, base_style.bg, honor) {
                    s = s.bg(c);
                }
                // SQ-0309 §3: apply the Glk style's typographic modifiers — the registry theme
                // slot's canonical modifiers (Emphasized→italic, Header→bold, …) plus any override
                // modifiers (garglk/game stylehints, populated later). Colours resolved above.
                let glk_mods =
                    crate::render::glk_theme_modifiers(scheme, false, glk) | slot.add_modifier;
                if !glk_mods.is_empty() {
                    s = s.add_modifier(glk_mods);
                }
            }
            // Glk hyperlink affordance: layer the themeable `hyperlink` colour
            // and an underline ON TOP of the styling. The underline is applied
            // here (not via `bits`) because `apply_text_style` has no underline
            // bit. Colour is gated on `honor`, matching the prior behaviour.
            if run.map(|r| r.link).unwrap_or(0) != 0 {
                if honor {
                    s = s.patch(scheme.theme.get("hyperlink").style);
                }
                s = s.add_modifier(ratatui::style::Modifier::UNDERLINED);
            }
            s
        };
        crate::render::draw_char_clipped(buf, col, y, ch, style, area);
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

/// Like `format_suggestion_line`, but horizontally scrolls the line to fit
/// `width` columns while keeping the highlighted `[bracketed]` entry visible.
///
/// When the full line fits, it is returned unchanged. Otherwise the window is
/// scrolled the minimum amount needed to bring the highlighted entry's right
/// edge into view, so Tabbing toward off-screen candidates pulls them into the
/// window instead of dropping the brackets off the right edge.
pub(crate) fn visible_suggestion_line(
    suggestions: &[String],
    highlight_idx: usize,
    width: usize,
) -> String {
    if suggestions.is_empty() || width == 0 {
        return String::new();
    }
    let idx = highlight_idx % suggestions.len();
    let line = format_suggestion_line(suggestions, highlight_idx);
    let total = line.chars().count();
    if total <= width {
        return line;
    }

    // Char span of the highlighted entry within the joined line: each preceding
    // entry contributes its word length plus a 2-space separator; the
    // highlighted entry itself adds 2 chars for the surrounding brackets.
    let hl_start: usize = suggestions[..idx]
        .iter()
        .map(|w| w.chars().count() + 2)
        .sum();
    let hl_end = hl_start + suggestions[idx].chars().count() + 2;
    // Scroll just enough to keep the highlighted entry's end on screen. For an
    // entry wider than the window, anchor on its start so the opening bracket
    // shows.
    let offset = if hl_end <= width {
        0
    } else if hl_end - hl_start >= width {
        hl_start
    } else {
        hl_end - width
    };
    line.chars().skip(offset).take(width).collect()
}

/// SQ-0542: the dim completion hint drawn immediately AFTER the typed input,
/// replacing the candidate bar for STORY-WORD completions.
///
/// The bar cost a row, and in the default inline-prompt mode that row came out of
/// the transcript viewport — so every keystroke that gained or lost a candidate
/// shifted the prompt row, and all the scrollback with it, by one line. A hint
/// rendered on the prompt row itself cannot move anything.
///
/// Story-word candidates ([`crate::complete::suggest`]) match by PREFIX, so the
/// candidate always extends what you typed and the hint is simply its tail:
/// typing `op` with `open` offered draws a dim `en` after the caret.
///
/// The COMMAND PALETTE is deliberately untouched: a line starting with the command
/// prefix keeps the bracketed candidate bar below the prompt (its names match by
/// substring, so they have no tail to show, and the palette is an explicit mode
/// where seeing every candidate at once is the point).
///
/// `None` when the caret is not at the end of the line (a hint mid-line reads as
/// text you typed), when focus is elsewhere or an overlay is up, or when the
/// candidate adds nothing — which is exactly the state right after Tab applied it,
/// so the hint clears itself without any extra bookkeeping.
pub(crate) fn ghost_completion(state: &AppState) -> Option<String> {
    if state.focus != Focus::Game || state.any_overlay_open() {
        return None;
    }
    if state.input.as_str().starts_with(state.config.command_prefix) {
        return None; // command palette — keeps its bar
    }
    if state.input.cursor < state.input.char_len() {
        return None;
    }
    let candidate = state.suggestions.get(state.suggestion_idx % state.suggestions.len().max(1))?;
    let partial = state.current_partial();
    if !candidate.to_lowercase().starts_with(&partial.to_lowercase()) {
        return None;
    }
    let hint: String = candidate.chars().skip(partial.chars().count()).collect();
    (!hint.is_empty()).then_some(hint)
}

/// The current inventory item list: the engine's live object-tree contents
/// when `player_obj` is locked and introspection is available, otherwise the
/// last parsed `inventory_fallback` list. Used by the inventory dock panel
/// (`render::inventory_dock`) and by the top-level render to size the dock.
pub fn inventory_items(
    player_obj: Option<u16>,
    inventory_fallback: &[String],
    introspect: Option<&dyn Introspect>,
) -> Vec<String> {
    match (player_obj, introspect) {
        (Some(obj), Some(intro)) => intro.contents(obj),
        _ => inventory_fallback.to_vec(),
    }
}

// ── Main render function ───────────────────────────────────────────────────────

/// A rendered transcript pass: `(scrollbar_drawn, max_scroll, links)` — whether a
/// scrollbar gutter was drawn, the largest meaningful `transcript_scroll`, and a
/// per-frame map from rendered cell `(col, row)` to Glk hyperlink value.
type TranscriptRender = (bool, u16, u16, Vec<((u16, u16), u32)>);

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
///   Renders the GAME pane. Returns `(scrollbar_drawn, max_scroll)`: whether the
///   transcript drew a scrollbar gutter (so the caller can exclude that column
///   from text selection) and the largest meaningful `transcript_scroll` value.
pub fn render_transcript(
    status: &StatusModel,
    // No longer used here: the inventory moved out of this pane into the
    // docked panel (`render::inventory_dock`), which sources its own items
    // via `inventory_items` at the top-level render. Kept so callers (the
    // window-tree walk in `screen.rs`) don't need their own plumbing change.
    _introspect: Option<&dyn Introspect>,
    state: &AppState,
    area: Rect,
    buf: &mut Buffer,
    game_input: Option<Style>,
) -> TranscriptRender {
    if area.height == 0 || area.width == 0 {
        return (false, 0, 0, Vec::new());
    }

    let normal_style = state.colors.theme.get("transcript").style;

    // ── Determine status and input heights based on border style ─────────────

    let status_style_kind = state.colors.status_header_style;
    let input_style_kind  = state.colors.input_line_style;

    // The status bar always shows for v3 (its automatic status line is valid).
    // For v4+/Glulx (HostManaged) the synthesized bar is removed entirely —
    // transient app feedback no longer reuses the score bar; it surfaces as a
    // top-right notification toast instead (SQ-0176).
    let status_visible = matches!(status, StatusModel::Classic { .. });

    // When boxed, status/input each take 3 rows; fall back to 1 if too small.
    // Gate on "any side present" (base OR per-side) so a style="none" + per-side
    // config (e.g. left/right-only) still boxes and draws its side bars.
    let status_boxed = status_visible && (status_style_kind != BorderStyle::None || state.colors.status_header_sides.any_on()) && area.height >= 5;
    let input_boxed  = (input_style_kind  != BorderStyle::None || state.colors.input_line_sides.any_on()) && area.height >= 5;
    let status_rows: u16 = if !status_visible { 0 } else if status_boxed { 3 } else { 1 };
    // Inline-prompt mode (`command_bar` off): no dedicated bottom bar — the live
    // input is drawn flush after the game's kept `>` in the transcript body, so
    // the whole bottom flows into the middle area (`input_rows == 0`).
    let input_rows:  u16 = if !state.config.command_bar { 0 } else if input_boxed { 3 } else { 1 };

    // ── Top row(s): status line ──────────────────────────────────────────────

    let status_region = Rect::new(area.x, area.y, area.width, status_rows.min(area.height));

    if status_boxed {
        // Draw a pane frame around the status region.
        let status_header = state.colors.theme.get("status_header").style;
        let frame = draw_framed(buf, status_region, state.colors.status_header_sides, &state.colors.status_header_glyphs, status_header, false);
        // Render status text into the inner content row.
        render_status_content(status, state, buf, frame.content);
    } else {
        render_status_content(status, state, buf, status_region);
    }

    if area.height < status_rows + 1 {
        return (false, 0, 0, Vec::new());
    }

    // ── Bottom row(s): input line ─────────────────────────────────────────────

    let input_region_top = area.bottom().saturating_sub(input_rows);
    let input_region = Rect::new(area.x, input_region_top, area.width, input_rows.min(area.height));

    // Only the command-bar mode draws the dedicated bottom input bar. In inline
    // mode (`input_rows == 0`) the live input is drawn by `render_middle` flush
    // after the last transcript row (the game's kept `>` prompt).
    if state.config.command_bar {
        if input_boxed {
            let input_line = state.colors.theme.get("input_line").style;
            let frame = draw_framed(buf, input_region, state.colors.input_line_sides, &state.colors.input_line_glyphs, input_line, false);
            render_input_content(state, buf, frame.content, normal_style, game_input);
        } else {
            render_input_content(state, buf, input_region, normal_style, game_input);
        }
    }

    // ── Middle area: transcript + inventory + suggestion ─────────────────────

    let middle_top = area.y + status_rows;
    let middle_bottom = input_region_top;
    if middle_top >= middle_bottom {
        return (false, 0, 0, Vec::new());
    }
    let middle_area = Rect::new(area.x, middle_top, area.width, middle_bottom - middle_top);
    render_middle(state, buf, middle_area, normal_style, game_input)
}

/// Choose the rect notification toasts anchor to: the story pane's inner
/// content rect when it has room for at least a 1-row strip, else the full
/// frame. `draw_frame` calls this with the story pane's content rect (as
/// drawn this frame) and the full terminal frame, so a toast is never lost
/// even in `Layout::TranscriptFull`-without-room edge cases or a terminal so
/// small the story pane collapses to zero content area. (SQ-0415)
pub fn notification_anchor_rect(story_area: Rect, full: Rect) -> Rect {
    if story_area.width >= 6 && story_area.height > 0 {
        story_area
    } else {
        full
    }
}

/// Draw the top-right notification toasts over `area` — normally the story
/// pane's content rect, or the full frame as a fallback (see
/// [`notification_anchor_rect`]).
///
/// Newest is drawn on top; each toast slides in from `area`'s right edge,
/// holds for a few seconds, and slides out (see [`crate::notify`]), clipped to
/// `area` throughout. Called last in `draw_frame` so toasts overlay the story
/// pane's own content (and anything else drawn under `area`). When animations
/// are disabled the toast simply appears and disappears on the same clock,
/// without sliding. (SQ-0176, SQ-0415)
pub fn render_notifications(buf: &mut Buffer, area: Rect, state: &AppState) {
    let notes = state.notifications.active();
    if notes.is_empty() || area.width < 6 || area.height == 0 {
        return;
    }
    let style = state.colors.theme.get("notification").style;
    let animate = state.config.animation.enabled;
    let easing = state.config.animation.easing;
    // A single-bordered box by default (SQ-0176); collapses to a 1-row strip only
    // if the border is themed off or there's no vertical room for a frame.
    let boxed = (state.colors.notification_style != BorderStyle::None
        || state.colors.notification_sides.any_on())
        && area.height >= 3;
    let box_h: u16 = if boxed { 3 } else { 1 };
    // Cap the inner text width (leave room for a space of padding each side).
    let max_inner = (area.width as usize).min(48).saturating_sub(if boxed { 2 } else { 0 });
    let right = area.right();

    // `active()` is oldest-first; draw the newest (last) in the top box.
    for (i, note) in notes.iter().rev().enumerate() {
        let top = area.y + i as u16 * box_h;
        if top + box_h > area.bottom() {
            break;
        }
        let reveal = note.reveal(animate, easing);
        let text = truncate_line(&note.text, max_inner.saturating_sub(2));
        let inner = format!(" {text} ");
        let inner_w = inner.chars().count() as u16;
        let box_w = inner_w + if boxed { 2 } else { 0 };
        // Slide in from the right: the box translates leftward from off the right
        // edge; columns past `right` clip naturally against the buffer bounds.
        let shown = (reveal * box_w as f64).round().clamp(0.0, box_w as f64) as u16;
        if shown == 0 {
            continue;
        }
        let box_left = right - shown;
        let box_region = Rect::new(box_left, top, box_w, box_h);
        if boxed {
            let frame = draw_framed(
                buf,
                box_region,
                state.colors.notification_sides,
                &state.colors.notification_glyphs,
                style,
                false,
            );
            draw_str_clipped(buf, frame.content.x, frame.content.y, &inner, style, frame.content);
        } else {
            draw_str_clipped(buf, box_left, top, &inner, style, box_region);
        }
    }
}

/// Draw the status bar into `region`.
///
/// Each segment in `state.colors.statusbar_layout` is resolved (placeholders
/// substituted, empty ones hidden), styled (base patched with the segment
/// style), and packed into left/center/right clusters. Transient app feedback
/// is no longer drawn here — it surfaces as a notification toast (SQ-0176).
fn render_status_content(
    status: &StatusModel,
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    let base = state.colors.theme.get("status_bar").style;
    let status_y = region.y;
    let w = region.width as usize;

    // Fill the row with the base style.
    for x in region.x..region.right() {
        if let Some(cell) = buf.cell_mut((x, status_y)) {
            cell.set_symbol(" ").set_style(base);
        }
    }

    // For HostManaged (v4+/Glulx) the synthesized bar is removed entirely; a
    // Classic v3 bar renders its automatic status line below.
    let (location, right_field) = match status {
        StatusModel::HostManaged => return,
        StatusModel::Classic { location, right } => (location.clone(), *right),
    };

    // Build the field values for this turn (classic automatic status line).
    let (score, moves, time) = match right_field {
        StatusField::ScoreTurns { score, turns } => (Some(score.to_string()), Some(turns.to_string()), None),
        StatusField::Time { hours, minutes } => (None, None, Some(format!("{:02}:{:02}", hours, minutes))),
    };
    let filter = match state.transcript_filter {
        TranscriptFilter::Both => String::new(),
        TranscriptFilter::Story => "[filter: story]".to_string(),
        TranscriptFilter::Meta => "[filter: meta]".to_string(),
    };
    let fields = StatusFields {
        location,
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
    state: &AppState,
    buf: &mut Buffer,
    region: Rect,
    normal_style: Style,
    game_input: Option<Style>,
) {
    if region.height == 0 || region.width == 0 {
        return;
    }
    // In read_char mode — or while a Glulx game waits on a timer/mouse/hyperlink
    // event only — the prompt is meaningless: hide it entirely.
    if state.char_mode || state.event_wait {
        return;
    }
    let w = region.width as usize;
    let input_y = region.y;

    // The "> " prompt and the typed text are separately styleable (patched over
    // the normal style, so an unset selector renders identically to before).
    // The game's current colour, when honoured, wins over the theme fields.
    let input_prompt = state.colors.theme.get("input_prompt").style;
    let input_text = state.colors.theme.get("input_text").style;
    let base_prompt = normal_style.patch(input_prompt);
    let base_text = normal_style.patch(input_text);
    let (prompt_style, text_style) = match game_input {
        Some(gs) => (base_prompt.patch(gs), base_text.patch(gs)),
        None => (base_prompt, base_text),
    };
    let prefix = format_input_line(""); // "> "
    let prefix_trunc = truncate_line(&prefix, w);
    draw_str_clipped(buf, region.x, input_y, prefix_trunc, prompt_style, region);
    let text_x = region.x + prefix_trunc.chars().count() as u16;
    let text_w = w.saturating_sub(prefix_trunc.chars().count());
    let input_trunc = truncate_line(state.input.as_str(), text_w);
    draw_str_clipped(buf, text_x, input_y, input_trunc, text_style, region);
    // Where the text actually landed, so a click can be mapped back to a caret index (SQ-0354).
    state.input_text_origin.set(Some((text_x, input_y)));

    // SQ-0542: the completion hint rides on this row, after the typed text, so it
    // never moves anything. Drawn BEFORE the caret so the caret can sit on its
    // first glyph (the fish/zsh look) rather than blanking it.
    let ghost = ghost_completion(state);
    if let Some(g) = &ghost {
        let gx = text_x + input_trunc.chars().count() as u16;
        let gw = region.right().saturating_sub(gx) as usize;
        let g_trunc = truncate_line(g, gw);
        draw_str_clipped(buf, gx, input_y, g_trunc, state.colors.theme.get("suggestion").style, region);
    }

    if state.focus == Focus::Game && !state.any_overlay_open() {
        // Draw the caret where it actually IS, not always after the last char (SQ-0354). Clamped to
        // the drawn text: a long line is truncated to fit, and the caret must not be painted past
        // what is on screen.
        let drawn = input_trunc.chars().count();
        let cursor_x = text_x + state.input.cursor.min(drawn) as u16;
        if cursor_x < region.right() {
            if let Some(cell) = buf.cell_mut((cursor_x, input_y)) {
                // Mid-line, underscore the char the caret sits ON rather than replacing it — the
                // text must stay readable while you edit it.
                let style = cursor_style(text_style, game_input);
                // Mid-line, or sitting on the ghost's first glyph (SQ-0542): keep
                // the symbol and just restyle, so the text — and the hint — stay
                // readable under the caret.
                if state.input.cursor < drawn || ghost.is_some() {
                    cell.set_style(style);
                } else {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
    }
}

/// Render the middle section: suggestion line (or search hint), transcript body.
/// Returns `(scrollbar_drawn, max_scroll, links)` — whether a scrollbar gutter
/// was drawn in the rightmost column, the largest meaningful `transcript_scroll`
/// value (total wrapped rows minus the viewport) so the caller can clamp it, and
/// a per-frame map from rendered cell `(col, row)` → Glk hyperlink value.
fn render_middle(
    state: &AppState,
    buf: &mut Buffer,
    area: Rect,
    normal_style: Style,
    game_input: Option<Style>,
) -> TranscriptRender {
    if area.height == 0 || area.width == 0 {
        return (false, 0, 0, Vec::new());
    }
    let w = area.width as usize;

    // The input_y used by the original code was area.bottom() - 1 of the *full* area;
    // here area is already the middle section, so its bottom is the boundary.
    let middle_bottom = area.bottom(); // exclusive

    // ── Suggestion line or search hint: one row above middle_bottom ──────────
    // When search is active, the search hint replaces the suggestion line.
    let suggestion_y = middle_bottom.saturating_sub(1);
    let has_search = state.search_query.is_some();
    // SQ-0542: only the COMMAND PALETTE still shows the candidate bar. Story-word
    // completions draw as a ghost tail on the prompt row itself
    // ([`ghost_completion`]) and reserve nothing — the bar's row used to come out
    // of the transcript viewport, so in inline-prompt mode every keystroke that
    // gained or lost a candidate shifted the prompt row and all the scrollback
    // with it. Palette names match by substring and have no tail to show, and the
    // palette is an explicit mode where seeing every candidate at once is the
    // point, so it keeps the bar (and its bounce) unchanged.
    let has_suggestions = state.focus == Focus::Game
        && !state.suggestions.is_empty()
        && !has_search
        && state.input.as_str().starts_with(state.config.command_prefix);

    // Optional box chrome for the auto-complete popup: mirrors the input-line
    // boxing (base OR any per-side on, and enough room). When enabled, the popup
    // becomes a 3-row framed mini-window; otherwise it stays the 1-row strip.
    let sug_style_kind = state.colors.suggestion_line_style;
    let suggestion_boxed = has_suggestions
        && (sug_style_kind != BorderStyle::None || state.colors.suggestion_line_sides.any_on())
        && area.height >= 5;
    let box_top = middle_bottom.saturating_sub(3);
    let suggestion = state.colors.theme.get("suggestion").style;

    if suggestion_boxed {
        // Draw a pane frame around the 3-row popup region, then render the
        // suggestion strip into the inner content row.
        let box_region = Rect::new(area.x, box_top, area.width, 3);
        let frame = draw_framed(buf, box_region, state.colors.suggestion_line_sides, &state.colors.suggestion_line_glyphs, suggestion, false);
        let content = frame.content;
        if content.height >= 1 && content.width >= 1 {
            let sug_line = visible_suggestion_line(&state.suggestions, state.suggestion_idx, content.width as usize);
            draw_str_clipped(buf, content.x, content.y, &sug_line, suggestion, content);
        }
    } else if has_search && area.height >= 2 && suggestion_y > area.y {
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
        let hint_style = suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, hint_trunc, hint_style, area);
    } else if has_suggestions && area.height >= 2 && suggestion_y > area.y {
        // Horizontally scroll so the highlighted entry stays on screen rather
        // than being clipped off the right edge.
        let sug_line = visible_suggestion_line(&state.suggestions, state.suggestion_idx, w);
        let sug_style = suggestion;
        draw_str_clipped(buf, area.x, suggestion_y, &sug_line, sug_style, area);
    }

    // ── Transcript body ───────────────────────────────────────────────────────
    if area.height < 2 {
        // Not enough room for transcript when there's a suggestion row.
        return (false, 0, 0, Vec::new());
    }

    let transcript_top = area.y;
    let transcript_bottom = if suggestion_boxed {
        box_top
    } else if (has_search || has_suggestions) && suggestion_y > area.y {
        suggestion_y
    } else {
        middle_bottom
    };
    // [more] pager (SQ-0404): reserve the bottom row for the prompt when active,
    // as long as at least one transcript row remains above it.
    let more_row = (state.pager.active && transcript_bottom > transcript_top + 1)
        .then_some(transcript_bottom - 1);
    let transcript_bottom = transcript_bottom - more_row.is_some() as u16;

    if transcript_top >= transcript_bottom {
        return (false, 0, 0, Vec::new());
    }
    let transcript_rows = (transcript_bottom - transcript_top) as usize;

    let images_enabled = state.game_picker.is_some();
    let frameless = state.config.v6_render == crate::config::V6RenderMode::Frameless;
    let char_px = state
        .game_picker
        .as_ref()
        .map(|p| {
            let f = p.font_size();
            (f.width, f.height)
        })
        .unwrap_or((1, 1));
    // Reserve the rightmost column of the body as the scrollbar gutter so text
    // never collides with the scrollbar. Wrap and clip to this narrower body.
    let body_area = if area.width >= 2 {
        Rect { width: area.width - 1, ..area }
    } else {
        area
    };
    // Effective scroll: the animated displayed offset (line-rounded) while a
    // smooth scroll is in flight, else the logical target. Clamped below.
    let effective_scroll = state.effective_transcript_scroll();

    // Wrapped-transcript cache (SQ-0305): re-wrapping the whole filtered history
    // and cloning every visible line/run/image is the dominant per-frame cost, so
    // do it only when an input the wrapped rows actually depend on changed. An
    // idle redraw or a scroll (transcript, width, filter, styles all unchanged)
    // reuses the cached rows and just re-windows them to the viewport below.
    let cache_stale = {
        let cache = state.transcript_wrap.borrow();
        match cache.as_ref() {
            None => true,
            // Field-by-field so the hot (cache-hit) path never clones the colour
            // scheme / room name — only a rebuild pays for that (see the key built
            // below; keep the two field lists in sync).
            Some(c) => {
                c.key.gen != state.transcript_gen
                    || c.key.len != state.transcript.len()
                    || c.key.filter != state.transcript_filter
                    || c.key.width != body_area.width
                    || c.key.images_enabled != images_enabled
                    || c.key.char_px != char_px
                    || c.key.frameless != frameless
                    || c.key.clear_anchor != state.clear_anchor
                    || c.key.room_name.as_deref() != state.current_room_name.as_deref()
                    || c.key.colors != state.colors
            }
        }
    };
    if cache_stale {
        let visible_indices = state.visible_transcript_indices();
        let filtered_lines: Vec<String> = visible_indices.iter().map(|&i| state.transcript[i].clone()).collect();
        let filtered_kinds: Vec<TranscriptKind> = visible_indices.iter().map(|&i| state.transcript_kinds.get(i).copied().unwrap_or(TranscriptKind::Story)).collect();
        // Resolve each logical line's text style ONCE, before wrapping. Story lines
        // run through the rule list (user → location → system → base); the other
        // kinds use their fixed per-category style. Resolving here (not per wrapped
        // fragment) keeps whole-line matching correct when a line wraps.
        let room_name = state.current_room_name.as_deref();
        let transcript_input = state.colors.theme.get("transcript_input").style;
        let transcript_meta = state.colors.theme.get("transcript_meta").style;
        let transcript_warning = state.colors.theme.get("transcript_warning").style;
        let filtered_styles: Vec<Style> = visible_indices
            .iter()
            .zip(filtered_kinds.iter())
            .map(|(&i, kind)| {
                if let Some(ov) = state.transcript_styles.get(i).copied().flatten() {
                    return ov;
                }
                match kind {
                    TranscriptKind::Story   => state.colors.resolve_story_style(&state.transcript[i], room_name),
                    TranscriptKind::Input   => transcript_input,
                    TranscriptKind::Meta    => transcript_meta,
                    TranscriptKind::Warning => transcript_warning,
                }
            })
            .collect();
        let filtered_runs: Vec<Vec<StyleRun>> = visible_indices
            .iter()
            .map(|&i| state.transcript_runs.get(i).cloned().unwrap_or_default())
            .collect();
        let filtered_para: Vec<ParaFmt> = visible_indices
            .iter()
            .map(|&i| state.transcript_para.get(i).copied().unwrap_or_default())
            .collect();
        // Inline images parallel the filtered lines, indexed by the SAME visible
        // indices. Bands are only emitted when a game Picker is present (images
        // enabled); `char_px` is the picker's cell pixel size for pixel-accurate fit.
        // v6 hybrid: the chrome frame is scaled by the letterbox factor, so the
        // story's inline pictures (drop-caps, room icons — authored in native
        // game pixels) must scale by the SAME factor to match it. Published
        // per-frame by the v6 Layered arm; 0 (unset) or 1 = no scaling.
        let img_scale = state.v6_image_scale.get();
        let img_scale = if img_scale > 1.0 { img_scale } else { 1.0 };
        // Frameless mode (SQ-0461) applies its OWN inline-image sizing (drop-caps
        // to ~3–4 rows, band art upscaled) and is the ONLY mode that draws
        // graphics-window CONTENT splashes inline — hybrid/raster render the
        // window canvas itself, so they must DROP `ContentSplash` entries here to
        // avoid double-rendering. Both branches leave hybrid/raster byte-identical.
        let filtered_images: Vec<Option<crate::inline_image::InlineImage>> = visible_indices
            .iter()
            .map(|&i| {
                let mut img = state.transcript_images.get(i).cloned().flatten();
                if let Some(im) = img.as_mut() {
                    if !frameless && im.source == crate::inline_image::ImageSource::ContentSplash {
                        return None; // hybrid/raster draw the window itself
                    }
                    if im.scaled.is_none() {
                        if frameless {
                            im.scaled = im.frameless_scaled(char_px, body_area.width);
                        } else if img_scale > 1.0 {
                            let (w, h) = (im.pixels.width().max(1), im.pixels.height().max(1));
                            im.scaled = Some((
                                (w as f32 * img_scale) as u32,
                                (h as f32 * img_scale) as u32,
                            ));
                        }
                    }
                }
                img
            })
            .collect();
        // Bound the inline-image protocol cache to the images present in the
        // filtered transcript, keyed by source Arc-ptr. Combined with the pinned
        // Arc in each cache value, this drops entries only once their image is
        // truly gone. Cached and re-applied every frame (below).
        let live_bands: std::collections::HashSet<usize> = filtered_images
            .iter()
            .flatten()
            .map(|img| std::sync::Arc::as_ptr(&img.pixels) as usize)
            .collect();
        let rows = wrap_lines_kinded(
            &filtered_lines,
            &filtered_kinds,
            &filtered_styles,
            &filtered_runs,
            &filtered_para,
            &filtered_images,
            char_px,
            images_enabled,
            true, // main transcript: left-margin images float, text wraps beside (SQ-0454)
            body_area.width,
        );
        // Map the screen-clear boundary (a full-transcript index) to a position in
        // the filtered line list, so top-anchoring works under any transcript filter.
        let clear_anchor_filtered = state
            .clear_anchor
            .map(|a| visible_indices.iter().filter(|&&i| i < a).count());
        let anchor_row = anchor_wrapped_rows(
            &filtered_lines,
            &filtered_kinds,
            &filtered_styles,
            &filtered_runs,
            &filtered_para,
            &filtered_images,
            char_px,
            images_enabled,
            true,
            body_area.width,
            clear_anchor_filtered,
        );
        *state.transcript_wrap.borrow_mut() = Some(crate::state::TranscriptWrapCache {
            key: crate::state::TranscriptWrapKey {
                gen: state.transcript_gen,
                len: state.transcript.len(),
                filter: state.transcript_filter,
                width: body_area.width,
                images_enabled,
                char_px,
                frameless,
                clear_anchor: state.clear_anchor,
                room_name: state.current_room_name.clone(),
                colors: state.colors.clone(),
            },
            rows,
            anchor_row,
            live_bands,
        });
    }
    let cache = state.transcript_wrap.borrow();
    let entry = cache.as_ref().expect("wrap cache populated above");
    // Per-frame eviction: bound the inline-image protocol cache to present images.
    state.inline_image_render.borrow_mut().retain_live(&entry.live_bands);
    // Window the cached rows to the visible viewport (cheap; no re-wrap). The
    // top-anchor only applies at the bottom, handled inside `window_wrapped_rows`.
    let (lines, total_rows, first_abs_row) =
        window_wrapped_rows(&entry.rows, entry.anchor_row, transcript_rows, effective_scroll);
    // Search highlight style: black text on yellow background.
    let search_highlight_style = Style::new().fg(Color::Black).bg(Color::Yellow);
    let query_lower = state.search_query.as_deref().map(|q| q.to_lowercase()).unwrap_or_default();

    // Per-frame map from rendered cell (col, row) → Glk hyperlink value, so a
    // mouse click can be hit-tested to its link (consumed downstream in the
    // click gate). Cells are in the story-pane frame, which equals the Glk
    // screen frame.
    let mut links: Vec<((u16, u16), u32)> = Vec::new();
    // The current game-set background band, carried across blank rows so the gaps
    // between a game's coloured paragraphs fill too (SQ-0263).
    let mut band_bg: Option<ratatui::style::Color> = None;
    let meta_marker = state.colors.theme.get("meta_marker").style;
    let warning_marker = state.colors.theme.get("warning_marker").style;

    for (i, wr) in lines.iter().enumerate() {
        let row_y = transcript_top + i as u16;
        if row_y >= transcript_bottom {
            break;
        }
        // Inline-image band row: blit the strip for this row instead of text.
        if crate::render::inline_image::try_blit_band_row(state, wr, body_area.x, body_area.width, row_y, buf) {
            continue;
        }
        // Meta/Warning reserve the 2-col gutter and draw their marker glyph;
        // Story/Input draw flush left. The text style was resolved per logical
        // line above and is carried on every wrapped row.
        let (gutter, marker_style) = match wr.kind {
            TranscriptKind::Meta    => (Some(state.symbols.meta_gutter), meta_marker),
            TranscriptKind::Warning => (Some(state.symbols.warning_gutter), warning_marker),
            TranscriptKind::Story | TranscriptKind::Input => (None, Style::default()),
        };
        let text_x = if let Some(glyph) = gutter {
            draw_str_clipped(buf, body_area.x, row_y, &glyph.to_string(), marker_style, body_area);
            body_area.x + META_GUTTER
        } else {
            body_area.x
        };
        let search = has_search.then_some((query_lower.as_str(), search_highlight_style));
        draw_str_runs(buf, text_x, row_y, &wr.text, wr.style, &wr.runs, search, body_area, &state.colors, state.config.honor_game_colours);

        // Record cell→link for every linked span on this row. `run.start/end`
        // are char offsets within `wr.text` (re-based by `rebase_runs`), and
        // `draw_str_runs` draws char index `j` at column `text_x + j`, so the
        // rendered cell for a char is exactly that column. Clip to the body.
        for run in &wr.runs {
            if run.link == 0 {
                continue;
            }
            for j in run.start..run.end {
                let col = text_x.saturating_add(j as u16);
                if col >= body_area.right() {
                    break;
                }
                links.push(((col, row_y), run.link));
            }
        }

        // Extend a game-set background so a coloured paragraph reads as a solid
        // band, not a ragged block that stops at the text (when the pane's own
        // background differs from the row's). A row whose trailing run set a
        // background fills its trailing space with it and opens/continues a band;
        // a BLANK row inside that band fills fully (so the gaps between a game's
        // black-on-white paragraphs are white too); a non-blank Default row closes
        // the band (its own theme-coloured text must stay legible, so it is left
        // untouched). Only when honouring game colours. (SQ-0263)
        if state.config.honor_game_colours {
            let row_bg = wr.runs.last().and_then(|last| {
                let rbg = crate::state::unpack_zcolour(last.bg);
                (!matches!(rbg, zvm::screen::ZColour::Default))
                    .then(|| crate::render::resolve_zcolour(rbg, &state.colors))
            });
            if let Some(bg) = row_bg {
                // A coloured row: fill trailing space and (re)open the band.
                let fill = Style::default().bg(bg);
                let start = text_x.saturating_add(wr.text.chars().count() as u16);
                for x in start..body_area.right() {
                    if let Some(cell) = buf.cell_mut((x, row_y)) {
                        cell.set_symbol(" ").set_style(fill);
                    }
                }
                band_bg = Some(bg);
            } else if wr.text.trim().is_empty() {
                // A blank row inside a coloured band: fill it whole with the band bg.
                if let Some(bg) = band_bg {
                    let fill = Style::default().bg(bg);
                    for x in body_area.x..body_area.right() {
                        if let Some(cell) = buf.cell_mut((x, row_y)) {
                            cell.set_symbol(" ").set_style(fill);
                        }
                    }
                }
            } else {
                // A non-blank Default row closes the band.
                band_bg = None;
            }
        }

        // Left-margin float (SQ-0454): blit the picture strip over the left
        // `cols` columns AFTER the row's (indented) text and any background fill,
        // so the image always wins. The prose already started past `indent`, so
        // it never collides with the picture.
        if let Some(float) = &wr.float {
            crate::render::inline_image::blit_float_row(state, float, body_area.x, body_area.width, row_y, buf);
        }
    }

    // ── Inline live input (command_bar off) ──────────────────────────────────
    // Draw the typed command + block cursor flush after the last transcript row
    // (the game's kept `>` prompt), so scrollback and the live line read as one
    // continuous prompt. Only when the bottom of the transcript is on screen
    // (effective_scroll == 0) so scrolled-up history is never overwritten.
    if !state.config.command_bar
        && !state.char_mode
        && !state.event_wait
        && state.focus == Focus::Game
        && !state.any_overlay_open()
        && effective_scroll == 0
        && !lines.is_empty()
    {
        // A tall margin float outlives the prose beside it: every wrapped row
        // below the game's `>` carries the float geometry but an EMPTY text (see
        // `WrappedRow::float`). The live input belongs on the PROMPT row, so walk
        // back over that text-less tail — otherwise the typed command and its
        // caret land at the left margin somewhere down the picture's flank, which
        // is exactly what Shogun's opening did beside the ship (SQ-0544).
        // Ordinary rows (no float, or a float row that still carries its text)
        // stop the walk immediately, so nothing else moves.
        let bottom_i = lines.len().min(transcript_rows) - 1;
        let mut last_i = bottom_i;
        while last_i > 0
            && lines[last_i].text.is_empty()
            && lines[last_i].band.is_none()
            && lines[last_i].float.is_some()
        {
            last_i -= 1;
        }
        let row_y = transcript_top + last_i as u16;
        let last = &lines[last_i];
        // Only draw when the true last wrapped row is the one visible at the
        // bottom, it fits inside the transcript region, and it is a text row
        // (never an inline-image band).
        // The scroll test still asks about the TRUE bottom row (is the end of the
        // transcript on screen?), not the prompt row the walk above settled on.
        if row_y < transcript_bottom
            && first_abs_row + bottom_i == total_rows.saturating_sub(1)
            && last.band.is_none()
        {
            // Flush after the last line's text — matching Task 3's flush command
            // echo. Use the SAME text_x the draw loop used for this row's kind:
            // Story/Input draw at body_area.x; Meta/Warning reserve the gutter.
            let input_text = state.colors.theme.get("input_text").style;
            let base_text = normal_style.patch(input_text);
            let text_style = match game_input {
                Some(gs) => base_text.patch(gs),
                None => base_text,
            };
            let row_text_x = match last.kind {
                TranscriptKind::Meta | TranscriptKind::Warning => body_area.x + META_GUTTER,
                TranscriptKind::Story | TranscriptKind::Input => body_area.x,
            };
            let start_col = row_text_x + last.text.chars().count() as u16;
            let avail = body_area.right().saturating_sub(start_col) as usize;
            let input_trunc = truncate_line(state.input.as_str(), avail);
            draw_str_clipped(buf, start_col, row_y, input_trunc, text_style, body_area);
            // Where the input text landed, so a click maps back to a caret index
            // (SQ-0354). The command-bar path sets this too; in inline mode this is
            // the only place it's set, so a click can find the line at all.
            state.input_text_origin.set(Some((start_col, row_y)));
            // SQ-0542: the completion hint, drawn on this very row after the typed
            // text. This is the mode the bounce came from — the old bar took its row
            // out of the transcript viewport, so the prompt row and every line above
            // it shifted on each keystroke that gained or lost a candidate. Drawn
            // before the caret so the caret can sit on its first glyph.
            let drawn = input_trunc.chars().count();
            let ghost = ghost_completion(state);
            if let Some(g) = &ghost {
                let gx = start_col + drawn as u16;
                let gw = body_area.right().saturating_sub(gx) as usize;
                let g_trunc = truncate_line(g, gw);
                draw_str_clipped(buf, gx, row_y, g_trunc, state.colors.theme.get("suggestion").style, body_area);
            }
            // Draw the caret where it actually IS, not always after the last char
            // (SQ-0354). Clamped to the drawn (truncated) text.
            let cursor_x = start_col + state.input.cursor.min(drawn) as u16;
            if cursor_x < body_area.right() {
                if let Some(cell) = buf.cell_mut((cursor_x, row_y)) {
                    let style = cursor_style(text_style, game_input);
                    // Mid-line, or over the ghost's first glyph: reverse-video the
                    // char the caret sits on (keep it readable); at the end of the
                    // line with no hint: draw the "_" block.
                    if state.input.cursor < drawn || ghost.is_some() {
                        cell.set_style(style);
                    } else {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
            }
        }
    }

    // Publish this frame's transcript geometry so the mouse handlers and the copy
    // path can map screen cells ↔ absolute wrapped rows. (SQ-0197)
    state.transcript_geom.set(Some(crate::clipboard::TranscriptGeom {
        area: body_area,
        first_abs_row,
        total_rows,
    }));
    if let Some(sel) = state.selection {
        let width = body_area.width;
        // Highlight the visible portion (reverse video) by absolute row.
        for (i, _wr) in lines.iter().enumerate() {
            let row_y = transcript_top + i as u16;
            if row_y >= transcript_bottom { break; }
            let abs = first_abs_row + i;
            for col in 0..width {
                if crate::clipboard::contains(width, sel, abs, col) {
                    if let Some(cell) = buf.cell_mut((body_area.x + col, row_y)) {
                        let s = cell.style();
                        cell.set_style(s.add_modifier(ratatui::style::Modifier::REVERSED));
                    }
                }
            }
        }
        // Extract the copy from the FULL wrapped set (off-screen rows included),
        // reusing the cached rows rather than re-wrapping. (SQ-0305)
        if sel.is_empty() {
            *state.selection_text.borrow_mut() = None;
        } else {
            let texts: Vec<&str> = entry.rows.iter().map(|r| r.text.as_str()).collect();
            *state.selection_text.borrow_mut() = Some(crate::clipboard::extract(&texts, width, sel));
        }
    }

    // ── Scrollbar (only when the content overflows the viewport) ──────────────
    let drew_scrollbar = total_rows > transcript_rows && area.width >= 2 && transcript_bottom > transcript_top;
    if drew_scrollbar {
        let start = total_rows
            .saturating_sub(effective_scroll as usize)
            .saturating_sub(transcript_rows);
        // Push the scrollbar past the right text margin so it sits flush against
        // the pane border — only the text is inset by the margin (SQ-0345). The
        // margin band was already painted blank by `reserve_text_margin`.
        let right_margin = state.text_margin_applied.get();
        let sb_area = Rect {
            x: area.right() - 1 + right_margin,
            y: transcript_top,
            width: 1,
            height: transcript_bottom - transcript_top,
        };
        crate::render::scroll::draw_scrollbar(
            buf,
            sb_area,
            total_rows,
            transcript_rows,
            start,
            state.colors.theme.get("scrollbar").style,
        );
    }
    // [more] pager prompt (SQ-0404): a reverse-video bar on the reserved row.
    if let Some(row) = more_row {
        let mp = state.colors.theme.get("more_prompt").style;
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, row)) {
                cell.set_symbol(" ").set_style(mp);
            }
        }
        let label = "[more]  Space/PgDn continue  Esc skip";
        for (x, ch) in (area.x + 1..area.right()).zip(label.chars()) {
            if let Some(cell) = buf.cell_mut((x, row)) {
                cell.set_symbol(&ch.to_string()).set_style(mp);
            }
        }
    }
    let max_scroll = total_rows.saturating_sub(transcript_rows).min(u16::MAX as usize) as u16;
    let total = total_rows.min(u16::MAX as usize) as u16;
    (drew_scrollbar, max_scroll, total, links)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::cpu::exec::Machine;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    // ── Pure helper tests (no Machine required) ──────────────────────────────

    /// Build a `Theme` with the given selectors' fg overridden (like a
    /// `style.toml` decl), so tests exercising render code migrated to
    /// `theme.get("<selector>")` (SQ-0309) can still inject a custom colour
    /// instead of mutating the (no-longer-read) legacy `ColorScheme` field.
    fn theme_with_overrides(overrides: &[(&str, ratatui::style::Color)]) -> crate::theme::resolve::Theme {
        let mut decls = std::collections::HashMap::new();
        for &(sel, fg) in overrides {
            decls.insert(sel.to_string(), crate::theme::registry::Delta { fg: Some(fg), ..crate::theme::registry::Delta::EMPTY });
        }
        crate::theme::resolve::resolve(
            &crate::theme::resolve::Roles::terminal_default(),
            &decls,
            &std::collections::HashMap::new(),
            &std::collections::HashMap::new(),
        )
    }

    /// A helper: read `row` of `buf` as a String across `[x0, x1)`.
    fn read_row(buf: &Buffer, row: u16, x0: u16, x1: u16) -> String {
        (x0..x1)
            .map(|x| buf.cell((x, row)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect()
    }

    #[test]
    fn notification_toast_is_a_right_anchored_bordered_box() {
        let mut state = AppState::default();
        // Disable animation so reveal snaps to 1 (fully shown) — deterministic.
        state.config.animation.enabled = false;
        state.notifications.push("[Saved as: foo]");

        // The full frame is wider than the story pane (SQ-0415: a map pane sits
        // to the right of it, cols 40..60) — proves the toast anchors to the
        // PANE's right edge, not the frame's.
        let full = Rect::new(0, 0, 60, 10);
        let story_area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Default is a single-bordered box: top border (row 0), content (row 1),
        // bottom border (row 2). The bracket pair is stripped for a clean toast:
        // inner is " Saved as: foo " (15 chars), box 17 wide, right-anchored to
        // col 40 (the pane's right edge), so it spans cols 23..40.
        let content = read_row(&buf, 1, 24, 39);
        assert_eq!(content, " Saved as: foo ", "content row, bracket-stripped + padded: {content:?}");
        // Borders drawn above and below (non-blank in the box columns).
        assert!(!read_row(&buf, 0, 23, 40).trim().is_empty(), "top border drawn");
        assert!(!read_row(&buf, 2, 23, 40).trim().is_empty(), "bottom border drawn");
        // Nothing drawn past the story pane into the map's columns — the toast
        // is clipped to the pane, not the wider frame.
        assert_eq!(read_row(&buf, 1, 40, 60).trim(), "", "toast is clipped to the story pane, not the map");
        // The notification style — the registry's `notification` selector, which
        // derives from the `accent` role reversed (cyan reverse-video) — is
        // applied to the content cells. (SQ-0309: was a baked black-on-cyan
        // Style; now REVERSED cyan fg, same visual result.)
        let cell = buf.cell((30, 1)).expect("content cell exists");
        let themed = state.colors.theme.get("notification").style;
        assert_eq!(Some(cell.fg), themed.fg, "toast uses the themed notification fg");
        assert_eq!(cell.modifier, themed.add_modifier, "toast uses the themed notification modifiers");
    }

    #[test]
    fn notification_toasts_stack_newest_on_top() {
        let mut state = AppState::default();
        state.config.animation.enabled = false;
        state.notifications.push("older");
        state.notifications.push("newer");

        // Same pane-narrower-than-frame setup as above (SQ-0415).
        let full = Rect::new(0, 0, 60, 12);
        let story_area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(full);
        render_notifications(&mut buf, story_area, &state);

        // Each toast is a 3-row box: newest in rows 0-2, older in rows 3-5.
        let newest_box = (0..3).map(|r| read_row(&buf, r, 0, 40)).collect::<String>();
        let older_box = (3..6).map(|r| read_row(&buf, r, 0, 40)).collect::<String>();
        assert!(newest_box.contains("newer"), "newest is in the top box");
        assert!(older_box.contains("older"), "older is pushed to the box below");
        // Nothing spills past the pane into the map columns on either box.
        let map_cols = (0..6).map(|r| read_row(&buf, r, 40, 60)).collect::<String>();
        assert_eq!(map_cols.trim(), "", "toasts stay within the story pane's width");
    }

    #[test]
    fn no_toasts_when_notifications_empty() {
        let state = AppState::default();
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_notifications(&mut buf, area, &state);
        // Nothing drawn: the top-right stays blank.
        assert_eq!(read_row(&buf, 0, 0, 40).trim(), "");
    }

    #[test]
    fn notification_anchor_falls_back_to_full_frame_when_pane_absent_or_tiny() {
        let full = Rect::new(0, 0, 80, 24);

        // A real story pane with room: anchors there, not the frame (SQ-0415).
        let roomy_pane = Rect::new(0, 0, 40, 24);
        assert_eq!(notification_anchor_rect(roomy_pane, full), roomy_pane);

        // No story pane at all (e.g. a layout/edge case with zero content rect):
        // falls back to the full frame so a toast is never lost.
        assert_eq!(notification_anchor_rect(Rect::default(), full), full);

        // A pane too narrow for even the 1-row/6-col minimum toast strip: also
        // falls back to the full frame.
        let tiny_pane = Rect::new(0, 0, 4, 24);
        assert_eq!(notification_anchor_rect(tiny_pane, full), full);

        // A pane with zero height (borders alone ate all the rows): falls back.
        let zero_height_pane = Rect::new(0, 0, 40, 0);
        assert_eq!(notification_anchor_rect(zero_height_pane, full), full);
    }

    #[test]
    fn input_uses_full_width_warning_wraps_like_meta() {
        let line = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        // Input: full width 8 (no gutter) → unsplit.
        let i = wrap_lines_kinded(&line, &[TranscriptKind::Input], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(i.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
        // Warning: wraps to width-2 = 6 like Meta; continuation gets 2-space
        // hanging indent (leading_spaces("abcdefgh")=0, .max(2)=2).
        let w = wrap_lines_kinded(&line, &[TranscriptKind::Warning], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(w.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdef", "  gh"]);
    }

    // ── Inline-image band wrapping ────────────────────────────────────────────

    fn dummy_img(w: u32, h: u32, align: crate::inline_image::ImageAlign) -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage { pixels: std::sync::Arc::new(image::RgbaImage::new(w, h)), align, scaled: None , margin_px: None, source: crate::inline_image::ImageSource::Story }
    }

    #[test]
    fn image_unit_expands_to_band_rows() {
        // Two text lines with an image unit between; image 16x24 px, cell 8x8 →
        // 2 cols x 3 rows band.
        let transcript = vec!["hi".to_string(), String::new(), "bye".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let runs = vec![Vec::new(); 3];
        let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp)), None];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        // 1 (hi) + 3 (band) + 1 (bye) = 5 rows.
        assert_eq!(rows.len(), 5);
        assert!(rows[0].band.is_none());
        assert_eq!(rows[1].band.as_ref().unwrap().rows, 3);
        assert_eq!(rows[1].band.as_ref().unwrap().row, 0);
        assert_eq!(rows[3].band.as_ref().unwrap().row, 2);
        assert_eq!(rows[4].text, "bye");
    }

    #[test]
    fn image_unit_emits_zero_rows_when_disabled() {
        let transcript = vec!["hi".to_string(), String::new()];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let images = vec![None, Some(dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp))];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), false, false, 40);
        assert_eq!(rows.len(), 1); // only "hi"
    }

    #[test]
    fn band_reflows_narrower_on_smaller_width() {
        // 800x400 px, cell 8x8: width 40 → 40x20; width 20 → 20x10.
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(800, 400, crate::inline_image::ImageAlign::InlineUp))];
        let wide = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        let narrow = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 20);
        assert_eq!(wide.len(), 20);
        assert_eq!(narrow.len(), 10);
    }

    #[test]
    fn margin_right_band_rows_run_0_to_rows_with_pinned_x_off() {
        // 16x24 px, cell 8x8 -> 2x3 cells (same geometry as
        // image_unit_expands_to_band_rows); MarginRight at width 40 pins
        // x_off = 40 - 2 = 38 on every one of the 3 band rows, with `row`
        // running 0, 1, 2 in order.
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight))];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows.len(), 3);
        for (expected_row, wr) in rows.iter().enumerate() {
            let band = wr.band.as_ref().unwrap();
            assert_eq!(band.row, expected_row as u16);
            assert_eq!(band.cols, 2);
            assert_eq!(band.rows, 3);
            assert_eq!(band.x_off, 38);
        }
    }

    #[test]
    fn margin_right_sets_x_offset() {
        let transcript = vec![String::new()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let images = vec![Some(dummy_img(16, 8, crate::inline_image::ImageAlign::MarginRight))]; // 2x1 cells
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows[0].band.as_ref().unwrap().x_off, 38); // 40 - 2
    }

    // ── Left-margin floats (SQ-0454) ─────────────────────────────────────────

    fn left_img(w: u32, h: u32, margin_px: Option<u32>) -> crate::inline_image::InlineImage {
        crate::inline_image::InlineImage {
            pixels: std::sync::Arc::new(image::RgbaImage::new(w, h)),
            align: crate::inline_image::ImageAlign::MarginLeft,
            scaled: None,
            margin_px,
            source: crate::inline_image::ImageSource::Story,
        }
    }

    #[test]
    fn float_indent_derives_from_width_with_gutter_when_no_margin() {
        // No margin_px: text offset = image cell width + a 1-column gutter.
        let img = left_img(16, 24, None); // 2 cols at cell width 8
        assert_eq!(float_text_indent(&img, 8, 2), 3);
    }

    #[test]
    fn float_indent_honours_margin_px_scaled() {
        // margin_px is in GAME pixels; scale 1 → 48px / 8 = 6 cols.
        let img = left_img(16, 24, Some(48));
        assert_eq!(float_text_indent(&img, 8, 2), 6);
        // With a 2x scaled request (scaled_w 32 / native 16), the game margin
        // scales too: 48 * 2 = 96px / 8 = 12 cols.
        let mut scaled = left_img(16, 24, Some(48));
        scaled.scaled = Some((32, 48));
        assert_eq!(float_text_indent(&scaled, 8, 4), 12);
    }

    #[test]
    fn float_indent_never_below_image_width() {
        // A tiny game margin can't pull text under the picture: indent ≥ cols.
        let img = left_img(64, 8, Some(1)); // margin 1px → 1 col, but 8 cols wide
        assert_eq!(float_text_indent(&img, 8, 8), 8);
    }

    #[test]
    fn float_start_falls_back_to_band_for_unsupported_or_wide() {
        // Inline (non-margin) alignment never floats.
        assert!(FloatState::start(&dummy_img(16, 24, crate::inline_image::ImageAlign::InlineUp), (8, 8), 40).is_none());
        // A left picture wider than ~half the viewport (25 cols of 40) → band.
        assert!(FloatState::start(&left_img(200, 8, None), (8, 8), 40).is_none());
        // A right picture that leaves no prose column (34 cols of 40 → reserve 35,
        // only 5 cols of text < the 8-col floor) → band.
        assert!(FloatState::start(&dummy_img(272, 8, crate::inline_image::ImageAlign::MarginRight), (8, 8), 40).is_none());
        // A normal left-margin picture floats: 16x24 px at cell 8x8 → 2x3 cells,
        // reserve 3 (cols + gutter), text pushed right (pad 3), image at x_off 0.
        let fs = FloatState::start(&left_img(16, 24, None), (8, 8), 40).unwrap();
        assert_eq!((fs.cols, fs.rows, fs.reserve, fs.pad, fs.x_off, fs.next_strip), (2, 3, 3, 3, 0, 0));
        // A right-margin picture (Shogun's opening) floats at the RIGHT: 16x24 →
        // 2x3 cells, reserve 3, text flush left (pad 0), image right-aligned
        // (x_off = 40 - 2 = 38).
        let fr = FloatState::start(&dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight), (8, 8), 40).unwrap();
        assert_eq!((fr.cols, fr.rows, fr.reserve, fr.pad, fr.x_off, fr.next_strip), (2, 3, 3, 0, 38, 0));
    }

    #[test]
    fn left_float_wraps_following_prose_beside_the_picture() {
        // A left-margin drop-cap (16x24 px → 2x3 cells, indent 3) followed by a
        // long paragraph: the image emits NO band rows of its own; the first 3
        // output rows carry the picture strip and are indented by 3, wrapping the
        // prose at 40-3=37; rows past the picture reclaim full width.
        let para = "word ".repeat(30);
        let transcript = vec![para.trim_end().to_string()];
        let images = [Some(left_img(16, 24, None))];
        // The image is a SEPARATE transcript unit before the prose.
        let mut full_transcript = vec![String::new()];
        full_transcript.extend(transcript);
        let full_images = vec![images[0].clone(), None];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&full_transcript, &kinds, &styles, &runs, &[], &full_images, (8, 8), true, true, 40);

        // No standalone band rows for a floated image.
        assert!(rows.iter().all(|r| r.band.is_none()), "a floated image emits no bands");
        let float_rows: Vec<&WrappedRow> = rows.iter().filter(|r| r.float.is_some()).collect();
        assert_eq!(float_rows.len(), 3, "one float strip per image cell-row");
        for (k, r) in float_rows.iter().enumerate() {
            let fb = r.float.as_ref().unwrap();
            assert_eq!((fb.row, fb.cols, fb.rows, fb.x_off), (k as u16, 2, 3, 0));
            if !r.text.is_empty() {
                assert!(r.text.starts_with("   "), "float-row prose indented by 3");
            }
            assert!(r.text.chars().count() <= 40, "drawn width fits the viewport");
        }
        // The float begins at the first output row (no prose preceded the image).
        assert!(rows[0].float.is_some());
        // Rows past the 3-row picture reclaim full width (no forced float indent).
        assert!(
            rows.iter().skip(3).any(|r| r.float.is_none() && !r.text.is_empty()),
            "prose past the picture flows full-width"
        );
    }

    #[test]
    fn right_float_wraps_prose_flush_left_and_pins_picture_right() {
        // SQ-0489: a MarginRight picture (Shogun's opening) followed by a long
        // paragraph. The image emits NO band rows of its own; the covered rows
        // keep prose FLUSH LEFT (no pad) but narrowed to width - reserve, and each
        // carries the picture strip pinned to the RIGHT (x_off = width - cols);
        // rows past the picture reclaim full width. (16x24 px → 2x3 cells,
        // reserve 3, x_off 38 in a 40-col viewport.)
        let para = "word ".repeat(30);
        let transcript = vec![String::new(), para.trim_end().to_string()];
        let images = vec![Some(dummy_img(16, 24, crate::inline_image::ImageAlign::MarginRight)), None];
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);

        assert!(rows.iter().all(|r| r.band.is_none()), "a floated image emits no bands");
        let float_rows: Vec<&WrappedRow> = rows.iter().filter(|r| r.float.is_some()).collect();
        assert_eq!(float_rows.len(), 3, "one float strip per image cell-row");
        for (k, r) in float_rows.iter().enumerate() {
            let fb = r.float.as_ref().unwrap();
            assert_eq!((fb.row, fb.cols, fb.rows, fb.x_off), (k as u16, 2, 3, 38), "strip pinned to the right edge");
            // Prose is flush left (no leading pad) and narrowed to 40 - 3 = 37.
            assert!(!r.text.starts_with(' '), "right-float prose stays flush left: {:?}", r.text);
            assert!(r.text.chars().count() <= 37, "right-float prose narrowed to width - reserve");
        }
        // Rows past the 3-row picture reclaim full width.
        assert!(
            rows.iter().skip(3).any(|r| r.float.is_none() && r.text.chars().count() > 37),
            "prose past the picture flows full-width"
        );
    }

    #[test]
    fn left_float_emits_leftover_strips_when_taller_than_text() {
        // A 1-col x 5-row picture beside a single short line: the line rides strip
        // 0 (indented by 2), and the remaining 4 strips render as empty float rows
        // so the whole picture still draws.
        let transcript = vec![String::new(), "hi".to_string()];
        let images = vec![Some(left_img(8, 40, None)), None]; // 1 col, 5 rows
        let kinds = vec![TranscriptKind::Story; 2];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert_eq!(rows.len(), 5, "5 output rows: 1 with text + 4 leftover strips");
        assert_eq!(rows[0].text, "  hi", "text indented by 2 (1 col + gutter)");
        for (k, r) in rows.iter().enumerate() {
            let fb = r.float.as_ref().expect("every row carries a strip");
            assert_eq!((fb.row, fb.cols, fb.rows), (k as u16, 1, 5));
            if k > 0 {
                assert_eq!(r.text, "", "leftover strip rows carry no text");
            }
        }
    }

    #[test]
    fn left_float_too_wide_renders_as_band() {
        // A left-margin image wider than half the viewport falls back to the
        // existing full-width band path (band Some, float None).
        let transcript = vec![String::new()];
        let images = vec![Some(left_img(400, 8, None))]; // 40 cols after fit-to-width
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        assert!(rows.iter().all(|r| r.float.is_none()), "too-wide image never floats");
        assert!(rows[0].band.is_some(), "it renders as a band instead");
    }

    #[test]
    fn non_prose_line_flushes_the_float_first() {
        // An app-generated Meta line between the picture and prose finishes the
        // picture (as leftover strips) before rendering at full width.
        let transcript = vec![String::new(), "note".to_string()];
        let images = vec![Some(left_img(8, 24, None)), None]; // 1 col, 3 rows
        let kinds = vec![TranscriptKind::Story, TranscriptKind::Meta];
        let styles = vec![Style::default(); 2];
        let runs = vec![Vec::new(); 2];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, true, 40);
        // 3 leftover float strips, then the meta line.
        assert_eq!(rows.len(), 4);
        for (k, r) in rows.iter().take(3).enumerate() {
            assert_eq!(r.float.as_ref().unwrap().row, k as u16);
            assert_eq!(r.text, "");
        }
        assert!(rows[3].float.is_none() && rows[3].band.is_none());
        assert_eq!(rows[3].kind, TranscriptKind::Meta);
    }

    #[test]
    fn float_disabled_keeps_left_margin_as_band() {
        // With left_float off (the secondary-buffer-window path), a MarginLeft
        // image renders as a band exactly as before.
        let transcript = vec![String::new()];
        let images = vec![Some(left_img(16, 24, None))];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![Vec::new()];
        let rows = wrap_lines_kinded(&transcript, &kinds, &styles, &runs, &[], &images, (8, 8), true, false, 40);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|r| r.band.is_some() && r.float.is_none()));
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
        // narrow width 6: right "XY" preserved at 4; center dropped; left
        // "abcdef" truncated into the 4 cols before it — one word, so a hard
        // break plus the §8.2.2.2 ellipsis ("abc…")
        let ops3 = pack_status_clusters(&[mk("abcdef", Align::Left), mk("cc", Align::Center), mk("XY", Align::Right)], 6);
        assert!(ops3.iter().all(|(_, t, _)| t != "cc"), "center dropped under pressure");
        let right3 = ops3.iter().find(|(x, _, _)| *x == 4).unwrap();
        assert_eq!(right3.1, "XY");
        let left3 = ops3.iter().find(|(x, _, _)| *x == 0).unwrap();
        assert_eq!(left3.1, "abc…"); // 4 cols before the right cluster, ellipsis included
    }

    #[test]
    fn status_truncation_breaks_at_the_last_space_with_an_ellipsis() {
        // ZMSD §8.2.2.2: "If the object's short name exceeds the available room
        // on the status line, the author suggests that an interpreter should
        // break it at the last space and append an ellipsis."
        assert_eq!(truncate_status_text("West of House", 13), "West of House", "fits → untouched");
        assert_eq!(truncate_status_text("West of House", 20), "West of House");
        assert_eq!(truncate_status_text("West of House", 12), "West of…", "breaks at the last space");
        assert_eq!(truncate_status_text("Cyclops Room", 8), "Cyclops…");
        assert_eq!(truncate_status_text("Antechamber", 6), "Antec…", "one long word → hard break");
        assert_eq!(truncate_status_text("Antechamber", 1), "…");
        assert_eq!(truncate_status_text("Antechamber", 0), "");
        for (name, w) in [("West of House", 12), ("Cyclops Room", 8), ("Antechamber", 6)] {
            assert!(truncate_status_text(name, w).chars().count() <= w, "never exceeds the budget");
        }
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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let row: String = (0..40u16)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        // filter indicator pinned right (default bar includes ` {filter}`).
        assert!(row.contains("[filter: story]"), "default bar must show the filter indicator: {:?}", row);
        // status row keeps the reversed-video base fill.
        assert!(buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn suggestions_render_boxed_when_styled() {
        // With the suggestion_line border configured, the auto-complete popup is
        // drawn as a framed mini-window: a border glyph appears and the suggestion
        // text renders inside it.
        use crate::render::paneframe::PaneSides;
        let machine = minimal_machine();
        let mut state = AppState::default();
        // SQ-0542: the bar is the COMMAND PALETTE's presentation now — a line
        // starting with the command prefix. Story-word completions ghost instead.
        state.input.set("/pan", true);
        state.suggestions = vec!["panh".into(), "panv".into()];
        state.suggestion_idx = 0;
        state.colors.suggestion_line_style = BorderStyle::Single;
        state.colors.suggestion_line_sides = PaneSides::all(BorderStyle::Single);

        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mut has_border = false;
        let mut has_text = false;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains('│') { has_border = true; }
            if row.contains("panh") { has_text = true; }
        }
        assert!(has_border, "boxed suggestion popup must draw a border glyph");
        assert!(has_text, "suggestion text must render inside the box");
    }

    #[test]
    fn suggestions_stay_inline_when_box_off() {
        // With the default (off) suggestion_line border, the popup stays the flat
        // one-row strip: the text renders but no box chrome is drawn.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.input.set("/pan", true);
        state.suggestions = vec!["panh".into(), "panv".into()];
        state.suggestion_idx = 0;

        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let mut has_border = false;
        let mut has_text = false;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains('│') { has_border = true; }
            if row.contains("panh") { has_text = true; }
        }
        assert!(!has_border, "inline suggestion strip must not draw box chrome");
        assert!(has_text, "suggestion text must still render inline");
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
    fn draw_str_runs_applies_span_modifier() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Modifier, Style}};
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let runs = vec![StyleRun { start: 2, end: 4, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]; // bold chars 2..4
        draw_str_runs(&mut buf, 0, 0, "abcdef", Style::default(), &runs, None, area, &crate::colors::ColorScheme::terminal_default(), false);
        assert!(!buf[(0, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(2, 0)].modifier.contains(Modifier::BOLD));
        assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));
        assert!(!buf[(4, 0)].modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn draw_str_runs_hyperlink_underlines_and_colors() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Modifier, Style}};
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let mut cs = crate::colors::ColorScheme::terminal_default();
        cs.theme = theme_with_overrides(&[("hyperlink", Color::Magenta)]);
        // chars 2..5 carry link 7 (bold too, to prove the link layers on top).
        let runs = vec![StyleRun { start: 2, end: 5, bits: 0x02, fg: 0, bg: 0, link: 7, glk_style: 0 }];
        draw_str_runs(&mut buf, 0, 0, "abcdefgh", Style::default(), &runs, None, area, &cs, true);
        for x in 2..5u16 {
            assert!(buf[(x, 0)].modifier.contains(Modifier::UNDERLINED), "linked cell {x} underlined");
            assert_eq!(buf[(x, 0)].fg, Color::Magenta, "linked cell {x} uses hyperlink fg");
            assert!(buf[(x, 0)].modifier.contains(Modifier::BOLD), "linked cell {x} keeps its bold");
        }
        // Unlinked neighbours: no underline, no hyperlink colour.
        assert!(!buf[(1, 0)].modifier.contains(Modifier::UNDERLINED));
        assert_ne!(buf[(1, 0)].fg, Color::Magenta);
        assert!(!buf[(5, 0)].modifier.contains(Modifier::UNDERLINED));
        assert_ne!(buf[(5, 0)].fg, Color::Magenta);
    }

    #[test]
    fn draw_str_runs_glk_style_slots_seed_and_gate() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White); // element = transcript White
        let mut cs = crate::colors::ColorScheme::terminal_default();
        // Seed buffer (row 0): Input(8) cyan, Subheader(4) green.
        cs.glk_styles[0][8] = Style::default().fg(Color::Cyan);
        cs.glk_styles[0][4] = Style::default().fg(Color::Green);

        let draw = |glk_style: u8, fg: u32, honor: bool| {
            let mut b = Buffer::empty(area);
            let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg, bg: 0, link: 0, glk_style }];
            draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, &cs, honor);
            b[(0, 0)].fg
        };

        // Input run, no game colour → input_text (cyan) in BOTH gate states.
        assert_eq!(draw(8, 0, false), Color::Cyan, "Input slot applies (honor off)");
        assert_eq!(draw(8, 0, true), Color::Cyan, "Input slot applies (honor on)");
        // Subheader run → transcript_location (green).
        assert_eq!(draw(4, 0, false), Color::Green, "Subheader slot applies");
        // Normal run (empty runs) → element base (white).
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut b, 0, 0, "abc", base, &[], None, area, &cs, false);
        assert_eq!(b[(0, 0)].fg, Color::White, "Normal → element base");

        // honor gate: a game-set red fg on an Input run — honor ON → game red wins
        // over the slot; honor OFF → game IGNORED, slot cyan shows.
        let red = crate::state::pack_zcolour(zvm::screen::ZColour::True24(0x00FF_0000));
        assert_eq!(draw(8, red, true), Color::Rgb(255, 0, 0), "honor ON: game colour wins over slot");
        assert_eq!(draw(8, red, false), Color::Cyan, "honor OFF: game ignored, slot wins");
    }

    #[test]
    fn glk_buffer_emphasized_renders_italic() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White);
        let cs = crate::colors::ColorScheme::terminal_default();

        let draw = |glk_style: u8| {
            let mut b = Buffer::empty(area);
            let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg: 0, bg: 0, link: 0, glk_style }];
            draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, &cs, false);
            b[(0, 0)].modifier
        };

        // Emphasized (glk_style 1) → registry theme's italic modifier.
        assert!(draw(1).contains(Modifier::ITALIC), "Emphasized run renders italic");
        // Normal (glk_style 0) → no italic.
        assert!(!draw(0).contains(Modifier::ITALIC), "Normal run has no italic");
    }

    #[test]
    fn glk_buffer_header_renders_bold() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 6, 1);
        let base = Style::new().fg(Color::White);
        let cs = crate::colors::ColorScheme::terminal_default();

        let mut b = Buffer::empty(area);
        let runs = vec![StyleRun { start: 0, end: 3, bits: 0, fg: 0, bg: 0, link: 0, glk_style: 3 }];
        draw_str_runs(&mut b, 0, 0, "abc", base, &runs, None, area, &cs, false);
        assert!(b[(0, 0)].modifier.contains(Modifier::BOLD), "Header run renders bold");
    }

    #[test]
    fn render_transcript_builds_cell_link_map() {
        use zvm::screen::ZColour;
        use ratatui::style::Modifier;
        let machine = minimal_machine();
        let mut state = AppState::default();
        // A linked line ("northgate" → link 42) followed by a plain line.
        state.push_transcript_runs("northgate", TranscriptKind::Story, &[(9, 0, ZColour::Default, ZColour::Default, 42, ParaFmt::default(), 0, false)]);
        state.push_transcript("plain text");
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        let (_sb, _ms, _total, links) = render_transcript(
            &crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None,
        );

        // Exactly the 9 linked chars map to 42; the plain line contributes none.
        assert_eq!(links.len(), 9, "one entry per linked char, none from plain text");
        assert!(links.iter().all(|(_, v)| *v == 42), "all entries carry the link value");
        // Every recorded cell is where a linked glyph actually rendered — proves
        // the map coordinates line up with the rendered cells.
        for &((cx, cy), _) in &links {
            let cell = buf.cell((cx, cy)).unwrap();
            assert_ne!(cell.symbol(), " ", "linked cell holds a glyph");
            assert!(cell.modifier.contains(Modifier::UNDERLINED), "linked cell underlined");
        }
    }

    #[test]
    fn game_background_fills_to_row_end() {
        // A short line with a game-set white background (like CM's black-on-white)
        // must paint the whole row width, not just behind the glyphs. (SQ-0263)
        use zvm::screen::ZColour;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript_runs(
            "hi", TranscriptKind::Story,
            &[(2, 0, ZColour::True24(0), ZColour::True24(0x00FF_FFFF), 0, ParaFmt::default(), 0, false)],
        );
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        let mut found = false;
        for y in 0..10u16 {
            if buf.cell((0, y)).unwrap().symbol() == "h" {
                found = true;
                assert_eq!(buf.cell((20, y)).unwrap().bg, white, "trailing space fills with the game bg");
                assert_eq!(buf.cell((37, y)).unwrap().bg, white, "fill reaches the body's right edge");
            }
        }
        assert!(found, "rendered the coloured line");
    }

    #[test]
    fn game_background_fills_blank_rows_within_a_band() {
        // Two black-on-white paragraphs separated by a blank line: the blank line
        // between them must also fill white, so the band is contiguous. (SQ-0263)
        use zvm::screen::ZColour;
        let white_run = |n: usize| (n, 0u8, ZColour::True24(0), ZColour::True24(0x00FF_FFFF), 0u32, ParaFmt::default(), 0, false);
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript_runs(
            "para1\n\npara2", TranscriptKind::Story,
            &[white_run(5), white_run(2), white_run(5)],
        );
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        let mut blank_white = false;
        for y in 0..10u16 {
            let text: String = (0..30).map(|x| buf.cell((x, y)).unwrap().symbol().to_owned()).collect();
            if text.trim().is_empty()
                && buf.cell((5, y)).unwrap().bg == white
                && buf.cell((20, y)).unwrap().bg == white
            {
                blank_white = true;
            }
        }
        assert!(blank_white, "the blank line inside the white band is filled white");
    }

    #[test]
    fn default_background_line_leaves_trailing_untouched() {
        // A normal (Default-bg) line must NOT get a trailing fill — the feature is
        // scoped to game-set backgrounds so ordinary games are unaffected. (SQ-0263)
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.push_transcript("hi");
        state.focus = Focus::Game;
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let white = ratatui::style::Color::Rgb(255, 255, 255);
        for y in 0..10u16 {
            if buf.cell((0, y)).unwrap().symbol() == "h" {
                assert_ne!(buf.cell((37, y)).unwrap().bg, white, "no game bg → no trailing fill");
            }
        }
    }

    #[test]
    fn draw_str_runs_empty_matches_clipped() {
        use ratatui::{buffer::Buffer, layout::Rect, style::Style};
        let area = Rect::new(0, 0, 10, 1);
        let mut a = Buffer::empty(area);
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut a, 0, 0, "hello", Style::default(), &[], None, area, &crate::colors::ColorScheme::terminal_default(), false);
        crate::render::draw_str_clipped(&mut b, 0, 0, "hello", Style::default(), area);
        assert_eq!(a, b, "empty runs render identically to draw_str_clipped");
    }

    #[test]
    fn draw_str_runs_empty_with_search_matches_highlighted() {
        use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};
        let area = Rect::new(0, 0, 20, 1);
        let base = Style::default();
        let hl = Style::new().fg(Color::Black).bg(Color::Yellow);
        let mut a = Buffer::empty(area);
        let mut b = Buffer::empty(area);
        draw_str_runs(&mut a, 0, 0, "the cat sat", base, &[], Some(("cat", hl)), area, &crate::colors::ColorScheme::terminal_default(), false);
        draw_str_highlighted(&mut b, 0, 0, "the cat sat", base, "cat", hl, area);
        assert_eq!(a, b, "empty runs + search render identically to draw_str_highlighted");
    }

    #[test]
    fn wrap_line_ranges_round_trips_word_wrap() {
        // Same row strings as wrap_line, plus correct source char ranges.
        let rows = wrap_line_ranges("AAAAA BBBBB", 5);
        assert_eq!(rows.iter().map(|(s, _, _)| s.clone()).collect::<Vec<_>>(),
                   wrap_line("AAAAA BBBBB", 5));
        // first row covers chars 0..5, second covers 6..11 (break space dropped)
        assert_eq!((rows[0].1, rows[0].2), (0, 5));
        assert_eq!((rows[1].1, rows[1].2), (6, 11));
    }

    #[test]
    fn wrap_lines_kinded_rebases_runs_per_row() {
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 6, end: 11, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]]; // bold "BBBBB"
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &[], &[], (1, 1), false, false, 5);
        // row 0 ("AAAAA", 0..5) → no runs; row 1 ("BBBBB", 6..11) → bold 0..5
        assert!(out[0].runs.is_empty());
        assert_eq!(out[1].runs, vec![StyleRun { start: 0, end: 5, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    // ── SQ-0538 / ZMSD §7.2.1: buffer_mode off ⇒ char-break, never word-wrap ──

    /// Helper: a line whose text is unbuffered from char `from` onwards.
    fn nowrap_para(from: u32) -> ParaFmt {
        ParaFmt { nowrap_from: Some(from), ..ParaFmt::default() }
    }

    fn wrapped(line: &str, width: u16, pf: ParaFmt) -> Vec<String> {
        let lines = vec![line.to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        wrap_lines_kinded(&lines, &kinds, &styles, &[], &[pf], &[], (1, 1), false, false, width)
            .into_iter()
            .map(|r| r.text)
            .collect()
    }

    #[test]
    fn unbuffered_line_breaks_after_last_char_that_fits() {
        // Buffered (the default) word-wraps; unbuffered breaks at the column.
        assert_eq!(wrapped("AAAAA BBBBB", 8, ParaFmt::default()), vec!["AAAAA", "BBBBB"]);
        assert_eq!(wrapped("AAAAA BBBBB", 8, nowrap_para(0)), vec!["AAAAA BB", "BBB"]);
    }

    #[test]
    fn unbuffered_long_word_splits_mid_word() {
        // A word longer than the width hard-breaks either way; the point is that
        // an unbuffered line never carries a partial word to the next row.
        assert_eq!(wrapped("Please wait...........", 10, nowrap_para(0)),
                   vec!["Please wai", "t.........", ".."]);
    }

    #[test]
    fn buffering_back_on_resumes_word_wrap() {
        // Same text, fully buffered again → ordinary word-wrap.
        assert_eq!(wrapped("Please wait...........", 10, ParaFmt::default()),
                   vec!["Please", "wait......", "....."]);
    }

    /// Documented mixed-line rule: rows that lie entirely inside the buffered
    /// prefix word-wrap; the first row reaching into the unbuffered region — and
    /// every row after it — char-breaks.
    #[test]
    fn mixed_line_word_wraps_until_buffering_turns_off() {
        // "one two " is buffered (8 chars); "xxxxxxxxxxxx" is not.
        let rows = wrapped("one two xxxxxxxxxxxx", 8, nowrap_para(8));
        assert_eq!(rows, vec!["one two", "xxxxxxxx", "xxxx"]);
    }

    #[test]
    fn unbuffered_wrap_ranges_stay_contiguous_for_selection() {
        // Selection/copy geometry (SQ-0197) re-bases runs from these ranges, so a
        // char-broken row must cover its source chars with no gaps.
        let rows = wrap_line_ranges_nw("AAAAA BBBBB", 8, Some(0));
        assert_eq!(rows.iter().map(|r| (r.1, r.2)).collect::<Vec<_>>(), vec![(0, 8), (8, 11)]);
        assert_eq!(rows.iter().map(|r| r.0.chars().count()).sum::<usize>(), 11,
                   "no break space is swallowed when buffering is off");
    }

    // ── SQ-0330: Glk paragraph layout (indent / para_indent / justification) ──

    #[test]
    fn centered_line_is_padded_to_centre_and_shifts_runs() {
        // "hi" (2 chars) centered in width 10 → (10-2)/2 = 4 leading spaces.
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 0, end: 2, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &para, &[], (1, 1), false, false, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "    hi", "text padded to centre");
        // The bold run must move right by the 4 padding columns so selection/copy
        // stays aligned with the drawn text.
        assert_eq!(out[0].runs, vec![StyleRun { start: 4, end: 6, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]);
    }

    #[test]
    fn right_flush_line_pads_all_slack() {
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 3, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        assert_eq!(out[0].text, "        hi", "right-flush pads to the right edge");
    }

    #[test]
    fn indented_paragraph_indents_every_wrapped_row() {
        // indent=2, width 8 → usable 6; "AAAAA BBBBB" wraps to "AAAAA"/"BBBBB",
        // each prefixed with 2 spaces.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 2, para_indent: 0, justify: 0, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 8);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "  AAAAA");
        assert_eq!(out[1].text, "  BBBBB");
    }

    #[test]
    fn para_indent_adds_to_first_row_only() {
        // indent=1, para_indent=2 → row 0 leads with 3 spaces, continuations with 1.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 1, para_indent: 2, justify: 0, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        assert_eq!(out[0].text, "   AAAAA", "row 0 gets indent+para_indent");
        assert_eq!(out[1].text, " BBBBB", "continuation gets only indent");
    }

    #[test]
    fn default_para_fmt_renders_identically_to_no_layout() {
        // The Z-machine path (default ParaFmt) must wrap byte-identically to the
        // pre-SQ-0330 behaviour: no padding, unshifted runs.
        let lines = vec!["AAAAA BBBBB".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let runs = vec![vec![StyleRun { start: 6, end: 11, bits: 0x02, fg: 0, bg: 0, link: 0, glk_style: 0 }]];
        let para = vec![ParaFmt::default()];
        let with_para = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &para, &[], (1, 1), false, false, 5);
        let without = wrap_lines_kinded(&lines, &kinds, &styles, &runs, &[], &[], (1, 1), false, false, 5);
        let texts_p: Vec<&str> = with_para.iter().map(|r| r.text.as_str()).collect();
        let texts_n: Vec<&str> = without.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts_p, texts_n);
        assert_eq!(with_para[1].runs, without[1].runs);
    }

    #[test]
    fn selection_over_centered_line_copies_the_right_characters() {
        // Simulate a copy: a selection picks columns [4, 6) of the padded row and
        // must yield "hi" (the visible text), because the padding is real row text
        // and the runs are shifted to match. A naive draw-time offset (not padding
        // the text) would make column 4 land on a space.
        let lines = vec!["hi".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        let para = vec![ParaFmt { indent: 0, para_indent: 0, justify: 2, nowrap_from: None }];
        let out = wrap_lines_kinded(&lines, &kinds, &styles, &[], &para, &[], (1, 1), false, false, 10);
        let row = &out[0].text;
        let sel: String = row.chars().skip(4).take(2).collect();
        assert_eq!(sel, "hi", "columns [4,6) of the padded row are the visible text");
        // Leading padding columns are spaces (selectable but blank).
        assert_eq!(row.chars().take(4).collect::<String>(), "    ");
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
        let result = wrap_lines_kinded(&lines, &kinds, &styles, &[], &[], &[], (1, 1), false, false, 5);
        let rows: Vec<&str> = result.iter().map(|wr| wr.text.as_str()).collect();
        assert_eq!(rows, vec!["hello", "world", "test", "short"]);
    }

    #[test]
    fn meta_lines_wrap_narrower_and_carry_kind() {
        // An 8-char wordless line: STORY fits in width 8; META wraps to width-2 = 6.
        // Continuation gets a 2-space hanging indent (leading_spaces("abcdefgh")=0,
        // .max(2)=2).
        let transcript = vec!["abcdefgh".to_string()];
        let st = [Style::default()];
        let m = wrap_lines_kinded(&transcript, &[TranscriptKind::Meta], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(m.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdef", "  gh"]);
        assert!(m.iter().all(|wr| matches!(wr.kind, TranscriptKind::Meta)));
        let s = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &st, &[], &[], &[], (1, 1), false, false, 8);
        assert_eq!(s.iter().map(|wr| wr.text.as_str()).collect::<Vec<_>>(), vec!["abcdefgh"]);
    }

    #[test]
    fn wrap_carries_logical_line_style_onto_every_row() {
        use ratatui::style::Color;
        // One logical line that wraps to 3 rows; its style must appear on all rows.
        let transcript = vec!["alpha beta gamma".to_string()];
        let styles = [Style::new().fg(Color::Magenta)];
        let rows = wrap_lines_kinded(&transcript, &[TranscriptKind::Story], &styles, &[], &[], &[], (1, 1), false, false, 5);
        assert!(rows.len() >= 3, "expected wrap to >= 3 rows, got {}", rows.len());
        assert!(rows.iter().all(|wr| wr.style.fg == Some(Color::Magenta)),
            "every wrapped row must carry the logical line's style");
    }

    #[test]
    fn visible_wrapped_lines_kinded_newest_at_bottom() {
        // 3 logical lines at width 10 = 3 display rows; scroll=0, rows=3
        let transcript = vec!["abc".to_string(), "def".to_string(), "ghi".to_string()];
        let kinds = vec![TranscriptKind::Story; 3];
        let styles = vec![Style::default(); 3];
        let (vis, total, first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, None);
        assert_eq!(vis.len(), 3);
        assert_eq!(total, 3, "total wrapped rows reported");
        assert_eq!(first, 0, "top visible row is absolute row 0");
        assert_eq!(vis[2].text, "ghi");
    }

    #[test]
    fn visible_wrapped_lines_kinded_scroll_offset() {
        // "hello world" wraps to ["hello", "world"] at width 5
        let transcript = vec!["hello world".to_string()];
        let kinds = vec![TranscriptKind::Story];
        let styles = vec![Style::default()];
        // scroll=1: end = 2-1=1, start = 1-1=0 → ["hello"]
        let (vis, total, first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 1, 1, 5, None);
        assert_eq!(vis[0].text, "hello");
        assert_eq!(total, 2, "both wrapped rows counted");
        assert_eq!(first, 0, "scroll=1 window starts at row 0");
        // scroll=0: end=2, start=1 → ["world"]
        let (vis2, _, first2) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 1, 0, 5, None);
        assert_eq!(vis2[0].text, "world");
        assert_eq!(first2, 1, "scroll=0 window starts at row 1");
    }

    #[test]
    fn visible_wrapped_lines_kinded_over_scroll_clamps_to_top() {
        // 5 logical lines, viewport 3 rows. Over-scrolling past the top must
        // keep showing the TOP 3 lines, not shrink the window from the bottom.
        let transcript: Vec<String> = (0..5).map(|i| format!("L{}", i)).collect();
        let kinds = vec![TranscriptKind::Story; 5];
        let styles = vec![Style::default(); 5];
        let (vis, total, _first) = visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 999, 10, None);
        assert_eq!(total, 5);
        assert_eq!(vis.len(), 3, "over-scroll still fills the viewport");
        assert_eq!(vis[0].text, "L0", "top line stays at the top");
        assert_eq!(vis[2].text, "L2");
    }

    #[test]
    fn format_input_line_prefix() {
        assert_eq!(format_input_line("open mailbox"), "> open mailbox");
        assert_eq!(format_input_line(""), "> ");
    }

    #[test]
    fn visible_wrapped_lines_kinded_top_anchors_after_clear() {
        // 5 lines, viewport 3. A clear boundary at index 3 → post-clear content
        // is lines 3,4 (2 lines); they pin to the TOP (2 rows returned, caller
        // leaves the rest blank) rather than bottom-sticking the full viewport.
        let transcript: Vec<String> = (0..5).map(|i| format!("L{}", i)).collect();
        let kinds = vec![TranscriptKind::Story; 5];
        let styles = vec![Style::default(); 5];
        let (vis, total, _first) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, Some(3));
        assert_eq!(total, 5);
        assert_eq!(vis.len(), 2, "only post-clear lines returned (top-anchored)");
        assert_eq!(vis[0].text, "L3");
        assert_eq!(vis[1].text, "L4");
        // No anchor → full viewport, bottom-stick (history stays in view).
        let (vis2, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, None);
        assert_eq!(vis2.len(), 3);
        assert_eq!(vis2[0].text, "L2", "bottom-stick pulls in the pre-clear line");
        // Scrolled up (scroll>0): anchor ignored so history is reachable.
        let (vis3, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 1, 10, Some(3));
        assert_eq!(vis3.len(), 3, "scrolled up ignores the clear anchor");
        assert_eq!(vis3[2].text, "L3");
        // Once post-clear content overflows the viewport, top-anchor stops
        // triggering and normal bottom-stick resumes (anchor at 0, 5 lines > 3).
        let (vis4, _, _) =
            visible_wrapped_lines_kinded(&transcript, &kinds, &styles, &[], &[], &[], (1, 1), false, 3, 0, 10, Some(0));
        assert_eq!(vis4.len(), 3);
        assert_eq!(vis4[2].text, "L4", "overflow → bottom-stick");
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

    /// Minimal v4 story (same minimal header as minimal_machine, version byte 4)
    /// so version() >= 4 — used to exercise the v4+ status-bar path.
    fn minimal_machine_v4() -> Machine {
        use zvm::memory::Memory;
        let mut buf = vec![0u8; 0x0800];
        buf[0x00] = 4; // version = 4
        buf[0x04] = 0x00; buf[0x05] = 0x40; // high mem
        buf[0x06] = 0x00; buf[0x07] = 0x40; // initial pc
        buf[0x0A] = 0x00; buf[0x0B] = 0x80; // dict
        buf[0x0C] = 0x01; buf[0x0D] = 0x00; // object table
        buf[0x0E] = 0x03; buf[0x0F] = 0x00; // globals
        buf[0x08] = 0x04; buf[0x09] = 0x00; // static base
        buf[0x18] = 0x00; buf[0x19] = 0x60; // abbrev table
        buf[0x0080] = 0; buf[0x0081] = 4; buf[0x0082] = 0; buf[0x0083] = 0; // dict
        let mem = Memory::new(buf).expect("minimal v4 story should be valid");
        Machine::new(mem)
    }

    #[test]
    fn v4_status_bar_does_not_render_synthesized_content() {
        // The synthesized babelmap status bar (room + turn counter) is removed for
        // v4+/HostManaged during normal play. The bar must be fully hidden when
        // there is no transient notification message.
        let machine = minimal_machine_v4();
        let mut state = AppState::default();
        state.current_room_name = Some("Outside".to_string());
        state.turns = 7;
        state.transcript = vec!["FIRSTLINE".to_string()];

        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Room name and turn counter must NOT appear anywhere in the buffer.
        let all_text: String = {
            let mut s = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    s.push(buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '));
                }
            }
            s
        };
        assert!(!all_text.contains("Outside"),
            "v4 must not render synthesized room name: {:?}", all_text);
        assert!(!all_text.contains("turn 7"),
            "v4 must not render synthesized turn counter: {:?}", all_text);
        // Transcript starts at y=0 (status bar occupies 0 rows).
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(top.contains("FIRSTLINE"),
            "transcript must begin at y=0 when HostManaged bar is hidden: {:?}", top);
    }

    #[test]
    fn v4_status_bar_collapsed_always_v3_always_shows() {
        let area = Rect::new(0, 0, 60, 5);

        // v4 + no message → top row is the transcript regardless of show_status_bar.
        // The synthesized bar is now unconditionally removed for HostManaged.
        let mv4 = minimal_machine_v4();
        let mut s = AppState::default();
        // show_status_bar=true is the default; the bar must still be hidden.
        s.current_room_name = Some("Outside".to_string());
        s.transcript = vec!["FIRSTLINE".to_string()];
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&mv4), None, &s, area, &mut buf, None);
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!top.contains("Outside"), "v4 synthesized bar must never render: {:?}", top);

        // v3 always shows its Classic status line (unaffected by this change).
        let mv3 = minimal_machine();
        let mut s3 = AppState::default();
        s3.show_status_bar = false;
        let mut buf3 = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&mv3), None, &s3, area, &mut buf3, None);
        let top3_reversed = buf3.cell((0, 0)).map(|c| c.modifier.contains(Modifier::REVERSED)).unwrap_or(false);
        assert!(top3_reversed, "v3 Classic status bar always renders regardless of show_status_bar");
    }

    // ── New: status-bar removal contract for HostManaged (v4+/Glulx) ──────────

    /// (a) HostManaged → bar hidden, no synthesized content (SQ-0176: transient
    /// feedback never reuses the score bar).
    #[test]
    fn host_managed_no_msg_status_row_hidden() {
        let mut state = AppState::default();
        state.current_room_name = Some("West of House".to_string());
        state.turns = 42;
        state.transcript = vec!["You are standing in front of a house.".to_string()];

        let area = Rect::new(0, 0, 60, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&StatusModel::HostManaged, None, &state, area, &mut buf, None);

        // Synthesized content must not appear anywhere.
        let all_text: String = {
            let mut s = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    s.push(buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '));
                }
            }
            s
        };
        assert!(!all_text.contains("West of House"),
            "HostManaged bar must not render room name when no status_msg: {:?}", all_text);
        assert!(!all_text.contains("turn 42"),
            "HostManaged bar must not render turn counter when no status_msg: {:?}", all_text);
        // Transcript flows from y=0 (status bar takes no rows).
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(top.contains("standing"),
            "transcript must start at y=0 (no status bar row): {:?}", top);
    }

    /// (c) Classic status line is unaffected — still always renders.
    #[test]
    fn classic_status_always_renders_unchanged() {
        // The Classic (v3) status line must be visible with its reversed background
        // and its location/score content regardless of show_status_bar.
        let machine = minimal_machine(); // v3 → Classic
        let mut state = AppState::default();
        state.show_status_bar = false; // toggling this must not hide the Classic bar
        state.transcript = vec!["some story text".to_string()];

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Top row (y=0) must have the reversed status bar background.
        assert!(
            buf.cell((0, 0)).unwrap().modifier.contains(Modifier::REVERSED),
            "Classic status bar must always render (reversed modifier) regardless of toggle"
        );
        // Transcript is below y=0, not at y=0.
        let top: String = (0..area.width)
            .map(|x| buf.cell((x, 0)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!top.contains("some story text"),
            "transcript must not occupy y=0 when Classic bar is visible: {:?}", top);
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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
    fn crash_lines_render_with_crash_style() {
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        let crash_style = state.colors.theme.get("transcript_crash").style;
        state.push_transcript_styled("*** VM FAULT ***", TranscriptKind::Warning, crash_style);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Single Warning-kind line → its gutter '!' marks the crash row.
        let y = (1u16..9)
            .find(|&y| buf.cell((0, y)).map(|c| c.symbol()) == Some("!"))
            .expect("crash line gutter '!' must appear in column 0");
        // The crash TEXT (past the 2-col gutter) must carry the crash style's fg
        // AND bold — proving the explicit style override applied. SQ-0309:
        // `transcript_crash` derives from the `alert` role same as the default
        // Warning yellow now (docs/design/2026-07-14-styling-role-redesign.md
        // §2), so BOLD is what distinguishes a crash line, not colour.
        assert_eq!(buf.cell((2, y)).unwrap().style().fg, crash_style.fg);
        assert_eq!(crash_style.fg, Some(Color::Yellow));
        assert!(crash_style.add_modifier.contains(ratatui::style::Modifier::BOLD));
        assert!(buf.cell((2, y)).unwrap().style().add_modifier.contains(ratatui::style::Modifier::BOLD));
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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
    fn render_story_location_line_is_accent_coloured() {
        // SQ-0309: `transcript_location` derives from the `accent` role, not
        // bold-only-white (docs/design/2026-07-14-styling-role-redesign.md §2).
        use ratatui::style::Color;
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.current_room_name = Some("West of House".to_string());
        state.push_transcript("West of House"); // Story
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let y = (1u16..9).find(|&y| {
            let row: String = (0..40u16).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect();
            row.starts_with("West of House")
        }).expect("location line must render");
        assert_eq!(
            buf.cell((0, y)).unwrap().style().fg,
            Some(Color::Cyan),
            "location header must carry the accent colour"
        );
    }

    #[test]
    fn render_transcript_input_and_transcript_lines() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.transcript = vec![
            "You are in a hall.".to_string(),
            "It is dark.".to_string(),
        ];
        state.input.set("open mailbox", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
    fn input_line_uses_game_colour() {
        use ratatui::style::Color;
        let mut state = AppState::default();
        state.config.honor_game_colours = true;
        state.input.set("x", true);
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        let game = Some(ratatui::style::Style::new().fg(Color::Cyan));
        render_input_content(&state, &mut buf, area, ratatui::style::Style::new(), game);
        // The "> " prompt occupies cols 0-1; the typed 'x' is at col 2.
        assert_eq!(buf.cell((2, 0)).unwrap().style().fg, Some(Color::Cyan),
            "typed input uses the game colour");
    }

    #[test]
    fn cursor_style_reverses_game_ink_but_falls_back_to_theme() {
        use ratatui::style::Color;
        // No game colour → the structural theme cursor (bare REVERSED), unchanged.
        assert_eq!(cursor_style(Style::new().fg(Color::Green), None), CURSOR_STYLE);
        // Game page set → reverse-video of the resolved input text so the caret is
        // visible on the recoloured page (SQ-0268). It carries the input fg/bg and
        // adds REVERSED (the terminal performs the swap).
        let text = Style::new().fg(Color::Rgb(0, 0, 0)).bg(Color::Rgb(255, 255, 255));
        let game = Some(Style::new().bg(Color::Rgb(255, 255, 255)));
        let cs = cursor_style(text, game);
        assert!(cs.add_modifier.contains(Modifier::REVERSED), "reversed block");
        assert_eq!(cs.fg, Some(Color::Rgb(0, 0, 0)), "keeps game fg (swapped by terminal)");
        assert_eq!(cs.bg, Some(Color::Rgb(255, 255, 255)), "keeps game bg (swapped by terminal)");
    }

    #[test]
    fn render_transcript_cursor_shown_when_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Cursor at position 4 ("> hi" = 4 chars → cursor at x=4).
        let cursor_cell = buf.cell((4, 4)).expect("cursor cell should exist");
        assert_eq!(cursor_cell.symbol(), " ", "block cursor is a reverse-video space at end of input");
        assert!(
            cursor_cell.modifier.contains(Modifier::REVERSED),
            "cursor cell should have REVERSED modifier"
        );
    }

    #[test]
    fn render_transcript_no_cursor_when_not_focused() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Map; // not focused on game

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Position x=4 should carry no cursor (no reverse-video block) in Map focus.
        let cell = buf.cell((4, 4)).expect("cell should exist");
        assert!(!cell.modifier.contains(Modifier::REVERSED), "no cursor when focus is Map");
    }

    #[test]
    fn render_transcript_no_cursor_when_overlay_open() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("hi", true);
        state.focus = Focus::Game; // focused on game, but overlay is open

        // Open the hotkey dialog — the simplest boolean overlay.
        state.overlays.hotkey_dialog = true;

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.focus = Focus::Game;
        state.input.set("zq", true);
        state.colors.theme = theme_with_overrides(&[("input_prompt", Color::Green), ("input_text", Color::Red)]);

        let area = Rect::new(0, 0, 40, 5);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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

    // ── Inline-prompt mode (command_bar off) ──────────────────────────────────

    #[test]
    fn inline_draws_flush_prompt_and_cursor_no_bar() {
        // command_bar off: the live input is drawn flush after the game's kept `>`
        // on the last transcript row (">look", no space), with a block cursor
        // right after it, and the dedicated bottom bar is gone.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = false;
        state.transcript = vec![
            "You are in a hall.".to_string(),
            ">".to_string(),
        ];
        state.input.set("look", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Locate the row that renders the flush prompt+input.
        let mut hit = None;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            if row.contains(">look") {
                hit = Some((y, row));
                break;
            }
        }
        let (y, row) = hit.expect("inline prompt row with '>look' must render");
        // Flush: '>' at col 0, "look" at cols 1..5, reverse-video block cursor at col 5.
        assert!(row.starts_with(">look"), "prompt+input must be flush: {:?}", row);
        assert_eq!(buf.cell((5, y)).unwrap().symbol(), " ", "block cursor sits right after '>look'");
        assert!(buf.cell((5, y)).unwrap().modifier.contains(Modifier::REVERSED));
        // The old dedicated bottom bar is dropped: the bottom row is blank.
        let bottom: String = (0..area.width)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(!bottom.contains('>'), "no dedicated bottom input bar in inline mode: {:?}", bottom);
    }

    #[test]
    fn command_bar_mode_still_draws_bottom_bar() {
        // command_bar on: the dedicated bottom bar renders "> look" as before,
        // unaffected by the inline path.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true;
        state.input.set("look", true);
        state.focus = Focus::Game;

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let bottom: String = (0..area.width)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(bottom.contains("> look"), "command-bar bottom row shows '> look': {:?}", bottom);
    }

    /// SQ-0542: a story-word completion must not move the input line — the bug that
    /// started this. The old candidate bar took its row out of the transcript
    /// viewport, so in inline-prompt mode (the default) every keystroke that gained
    /// or lost a candidate shifted the prompt row and every scrollback line with it,
    /// making the input bounce as you typed at the bottom of the pane.
    ///
    /// Renders the SAME state twice, once with candidates and once without, and
    /// requires the two frames to differ ONLY by the ghost tail: same prompt row,
    /// same scrollback, nothing reserved.
    #[test]
    fn story_word_completion_ghosts_without_moving_the_input_line() {
        let machine = minimal_machine();
        let render = |sugs: Vec<String>| -> Vec<String> {
            let mut state = AppState::default();
            state.config.command_bar = false; // inline prompt: the mode that bounced
            state.transcript = (0..50).map(|i| format!("L{i}")).collect();
            state.input.set("op", true);
            state.focus = Focus::Game;
            state.suggestions = sugs;
            let area = Rect::new(0, 0, 40, 12);
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                        .collect::<String>()
                })
                .collect()
        };
        let without = render(Vec::new());
        let with = render(vec!["open".to_string(), "operate".to_string()]);

        let prompt_row = |rows: &[String]| rows.iter().position(|r| r.contains("op")).expect("prompt row");
        assert_eq!(
            prompt_row(&without),
            prompt_row(&with),
            "the input line must not move when a completion appears\n  without: {:?}\n  with:    {:?}",
            without,
            with
        );
        // Every row above the prompt is untouched: no scrollback shifted, nothing reserved.
        let p = prompt_row(&without);
        for y in 0..p {
            assert_eq!(without[y], with[y], "row {y} shifted when the completion appeared");
        }
        // And the hint itself rides on the prompt row as the candidate's tail.
        assert!(with[p].contains("open"), "the ghost tail completes the typed word: {:?}", with[p]);
        assert!(!without[p].contains("open"), "no hint without candidates: {:?}", without[p]);
        // Nothing is drawn below the prompt row in either frame.
        for (y, row) in with.iter().enumerate().skip(p + 1) {
            assert!(row.trim().is_empty(), "row {y} below the prompt must stay empty: {row:?}");
        }
    }

    /// SQ-0542: the hint is a tail, so it only makes sense at the end of the line.
    /// Mid-line it would read as text you had typed.
    #[test]
    fn ghost_completion_hidden_when_the_caret_is_mid_line() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("op", true);
        state.suggestions = vec!["open".to_string()];
        assert_eq!(ghost_completion(&state).as_deref(), Some("en"), "at end of line the tail shows");
        state.input.home();
        assert_eq!(ghost_completion(&state), None, "mid-line the hint is suppressed");
    }

    /// SQ-0542: once Tab has applied the candidate the input already IS the word, so
    /// the tail is empty and the hint clears itself — no separate "applied" flag to
    /// keep in sync.
    #[test]
    fn ghost_completion_clears_once_the_candidate_is_applied() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.suggestions = vec!["open".to_string()];
        state.input.set("op", true);
        assert_eq!(ghost_completion(&state).as_deref(), Some("en"));
        state.input.set("open", true); // what Tab leaves behind
        assert_eq!(ghost_completion(&state), None);
    }

    /// SQ-0542: the command palette is deliberately untouched — its names match by
    /// substring and have no tail to show, so it keeps the candidate bar.
    #[test]
    fn ghost_completion_never_applies_to_the_command_palette() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("/set", true);
        state.suggestions = vec!["set-v6-render".to_string()];
        assert_eq!(ghost_completion(&state), None, "the palette keeps its bar, not a ghost");
    }

    /// SQ-0542: only the last WORD is completed, so a hint after an earlier word
    /// completes that word's tail and never re-completes the whole line.
    #[test]
    fn ghost_completion_completes_only_the_word_being_typed() {
        let mut state = AppState::default();
        state.focus = Focus::Game;
        state.input.set("take lan", true);
        state.suggestions = vec!["lantern".to_string()];
        assert_eq!(ghost_completion(&state).as_deref(), Some("tern"));
    }

    #[test]
    fn inline_input_suppressed_when_scrolled_up() {
        // command_bar off but scrolled up (effective_transcript_scroll > 0): the
        // live input must NOT be drawn, so scrolled-up history is never clobbered.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = false;
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();
        state.input.set("SECRETCMD", true);
        state.focus = Focus::Game;
        state.transcript_scroll = 5; // scrolled up from the bottom
        assert!(state.effective_transcript_scroll() > 0, "test must actually be scrolled up");

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            assert!(!row.contains("SECRETCMD"), "scrolled-up must not draw the live input: {:?}", row);
        }
    }

    #[test]
    fn more_pager_prompt_drawn_only_when_active() {
        // SQ-0404: an active pager reserves a bottom row for the `[more]` prompt.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = (0..50).map(|i| format!("L{i}")).collect();
        let area = Rect::new(0, 0, 40, 12);

        let has_more = |state: &AppState| {
            let mut buf = Buffer::empty(area);
            render_transcript(&crate::session::status_model_from_machine(&machine), None, state, area, &mut buf, None);
            (0..area.height).any(|y| {
                let row: String = (0..area.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                    .collect();
                row.contains("[more]")
            })
        };

        state.pager.active = false;
        assert!(!has_more(&state), "no prompt when the pager is inactive");
        state.pager.active = true;
        assert!(has_more(&state), "the [more] prompt shows on the reserved row when active");
    }

    #[test]
    fn render_transcript_shows_scrollbar_when_overflowing() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        // Far more lines than the viewport → scrollbar must appear.
        state.transcript = (0..50).map(|i| format!("L{}", i)).collect();
        state.colors.theme = theme_with_overrides(&[("scrollbar", Color::Magenta)]);

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // The scrollbar gutter is the rightmost column; its thumb glyph is '█'.
        let mut thumb_fg = None;
        let gutter: String = (0..area.height)
            .map(|y| {
                let c = buf.cell((area.width - 1, y));
                if let Some(cell) = c {
                    if cell.symbol().starts_with('█') { thumb_fg = Some(cell.fg); }
                }
                c.map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')
            })
            .collect();
        assert!(gutter.contains('█'), "scrollbar thumb should render in the rightmost column: {:?}", gutter);
        assert_eq!(thumb_fg, Some(Color::Magenta), "scrollbar is styleable via the `scrollbar` selector");
    }

    #[test]
    fn render_transcript_no_scrollbar_when_fits() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.transcript = vec!["only one line".to_string()];

        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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

    // ── visible_suggestion_line (horizontal scroll) tests ────────────────────
    #[test]
    fn visible_suggestion_line_returns_full_when_it_fits() {
        let sug = vec!["north".to_string(), "south".to_string()];
        let full = format_suggestion_line(&sug, 0);
        // Plenty of width: unchanged.
        assert_eq!(visible_suggestion_line(&sug, 0, 80), full);
    }

    #[test]
    fn visible_suggestion_line_no_scroll_when_highlight_near_start() {
        let sug = vec!["aa".to_string(), "bb".to_string(), "cccccccc".to_string()];
        // Full line: "[aa]  bb  cccccccc" (18 chars). Width 10 overflows, but the
        // highlighted entry sits at the start, so no scroll is needed.
        let out = visible_suggestion_line(&sug, 0, 10);
        assert!(out.starts_with("[aa]"), "highlight at start stays visible: {out:?}");
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn visible_suggestion_line_scrolls_to_keep_highlight_visible() {
        let sug = vec!["aaaa".to_string(), "bbbb".to_string(), "cccc".to_string()];
        // Full line: "aaaa  bbbb  [cccc]" (18 chars). At width 10 the highlighted
        // last entry would be clipped off the right — it must scroll into view.
        let out = visible_suggestion_line(&sug, 2, 10);
        assert!(out.contains("[cccc]"), "highlighted entry must stay on screen: {out:?}");
        assert_eq!(out.chars().count(), 10);
    }

    #[test]
    fn visible_suggestion_line_entry_wider_than_window_shows_opening_bracket() {
        let sug = vec!["xx".to_string(), "supercalifragilistic".to_string()];
        // The highlighted entry alone exceeds the window; anchor on its start so
        // the opening bracket is visible.
        let out = visible_suggestion_line(&sug, 1, 8);
        assert!(out.starts_with("[super"), "anchors on the opening bracket: {out:?}");
        assert_eq!(out.chars().count(), 8);
    }

    #[test]
    fn render_transcript_shows_suggestion_line_above_input() {
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.focus = Focus::Game;
        // SQ-0542: the bar belongs to the command palette now.
        state.input.set("/set", true);
        state.suggestions = vec!["set-v6-render".to_string()];
        state.suggestion_idx = 0;

        // 10-row area: row 0=status, rows 1..7=transcript, row 8=suggestion, row 9=input
        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Row 9 (bottom) must contain the input.
        let input_row: String = (0..40u16)
            .map(|x| buf.cell((x, 9)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(input_row.contains("> /set"), "input row: {:?}", input_row);

        // Row 8 must contain the suggestion.
        let sug_row: String = (0..40u16)
            .map(|x| buf.cell((x, 8)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
            .collect();
        assert!(sug_row.contains("set-v6-render"), "suggestion row: {:?}", sug_row);
    }

    #[test]
    fn transcript_color_override_paints_line_in_its_style() {
        use ratatui::style::{Color, Style};
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.push_transcript_styled("connector sample", TranscriptKind::Meta, Style::new().fg(Color::Cyan));
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
        // Find a cell of the line and assert its fg is Cyan (the override), not transcript_meta.
        let found = (0..area.height).any(|y| (0..area.width).any(|x| {
            let c = &buf[(x, y)];
            c.symbol().starts_with('c') && c.style().fg == Some(Color::Cyan)
        }));
        assert!(found, "color override paints the line in its style");
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

        // czech.z5 is a v5 story → HostManaged → synthesized bar is removed.
        // Without a status_msg the bar occupies 0 rows; y=0 is transcript content,
        // not a reversed-video status line.
        let state = AppState::default();
        let area = Rect::new(0, 0, 80, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        // Top row must NOT have the reversed modifier (the synthesized bar is gone).
        let top_has_reversed = (0..80u16).all(|x| {
            buf.cell((x, 0))
                .map(|c| c.modifier.contains(Modifier::REVERSED))
                .unwrap_or(false)
        });
        assert!(
            !top_has_reversed,
            "v5 HostManaged story must not show reversed status bar when no status_msg"
        );
    }

    // ── Inventory: no longer rendered in-pane ─────────────────────────────────

    #[test]
    fn render_transcript_never_shows_inventory_strip() {
        // The inventory moved to the docked panel (render::inventory_dock); this
        // pane must never draw an "Inv:" strip regardless of show_inventory.
        let machine = minimal_machine();
        let mut state = AppState::default();
        state.show_inventory = true;
        state.inventory_fallback = vec!["brass lamp".to_string(), "sword".to_string()];

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

        let found_inv = (0..10u16).any(|y| {
            let row: String = (0..40u16)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' '))
                .collect();
            row.contains("Inv:")
        });
        assert!(!found_inv, "Inv: strip must not appear inside the transcript pane");
    }

    #[test]
    fn inventory_items_live_vs_fallback() {
        // player_obj known + introspect available → not exercised here (no fake
        // Introspect impl in this module); covered by the (player_obj, None) and
        // (None, _) fallback arms.
        let items = vec!["brass lamp".to_string()];
        assert_eq!(inventory_items(None, &items, None), items);
        assert_eq!(inventory_items(Some(7), &items, None), items);
        assert!(inventory_items(None, &[], None).is_empty());
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
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
            render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
        state.config.command_bar = true; // this test exercises the dedicated bottom bar
        state.input.set("go north", true);
        state.focus = Focus::Game;

        // input_line_style defaults to None
        assert!(
            matches!(state.colors.input_line_style, BorderStyle::None),
            "default input_line_style must be None"
        );

        let area = Rect::new(0, 0, 40, 10);
        let mut buf = Buffer::empty(area);
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);

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
        use crate::render::paneframe::{draw_pane_frame, BorderStyle, PaneGlyphs};
        use ratatui::style::Style;

        // Resolve a color scheme with none borders (simulate `map_border = none`).
        let area = Rect::new(0, 0, 20, 10);
        let mut buf_map = Buffer::empty(area);
        let frame = draw_pane_frame(&mut buf_map, area, BorderStyle::None, &PaneGlyphs::default(), Style::default());

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
        let story_frame = draw_pane_frame(&mut buf_story, area, BorderStyle::None, &PaneGlyphs::default(), Style::default());
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
        render_transcript(&crate::session::status_model_from_machine(&machine), None, &state, area, &mut buf, None);
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

    #[test]
    fn hanging_indent_wraps_continuations() {
        // A 2-space-indented line longer than width wraps with continuations
        // indented 2 spaces.
        let line = "  abcd efgh ijkl mnop";
        let rows = wrap_line_hanging(line, 10, 2);
        assert!(rows.len() >= 2);
        assert!(rows[0].starts_with("  abcd"), "first row keeps original indent");
        for cont in &rows[1..] {
            assert!(cont.starts_with("  "), "continuation '{cont}' is indented 2");
        }
    }

    // ── Engine-abstraction equivalence (3b-i) ─────────────────────────────────
    //
    // These prove the new ScreenModel-fed render path carries exactly the same
    // facts the old machine-fed path read, so output stays byte-identical.

    /// The v3 status model carries the same location + score/turns the render
    /// path formerly read from `machine.status_line()`.
    #[test]
    fn status_model_mirrors_v3_status_line() {
        let m = minimal_machine(); // v3
        let model = crate::session::status_model_from_machine(&m);
        let sl = m.status_line();
        match model {
            StatusModel::Classic { location, right } => {
                assert_eq!(location, sl.location);
                match (right, sl.right) {
                    (
                        StatusField::ScoreTurns { score, turns },
                        zvm::screen::StatusRight::ScoreTurns { score: s2, turns: t2 },
                    ) => assert_eq!((score, turns), (s2, t2)),
                    (
                        StatusField::Time { hours, minutes },
                        zvm::screen::StatusRight::Time { hours: h2, minutes: m2 },
                    ) => assert_eq!((hours, minutes), (h2, m2)),
                    _ => panic!("right-field variant mismatch"),
                }
            }
            other => panic!("v3 must yield Classic, got {other:?}"),
        }
    }

    /// A v4+ machine has no automatic status line in the model (the app draws
    /// its own room/turn info), matching the old `version() >= 4` branch.
    #[test]
    fn status_model_is_host_managed_for_v4() {
        let m = minimal_machine_v4();
        assert_eq!(crate::session::status_model_from_machine(&m), StatusModel::HostManaged);
    }

    /// Painting the engine upper window then rendering through the ScreenModel
    /// grid reproduces those cells exactly (the v4+ upper-window render path).
    #[test]
    fn upper_window_model_render_reproduces_painted_grid() {
        use crate::render::paneframe::{BorderStyle, PaneSides};
        let mut m = minimal_machine_v4();
        m.screen.upper.resize(1, 3);
        m.screen.upper.put(1, 1, 'A', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        m.screen.upper.put(1, 3, 'Z', 0, zvm::screen::ZColour::Default, zvm::screen::ZColour::Default);
        m.screen.upper_window_rows = 1;

        let model = crate::session::screen_model_from_machine(&m);
        let grid = model.grid().expect("model carries a grid");

        let mut colors = crate::colors::ColorScheme::terminal_default();
        colors.virtual_window_border = BorderStyle::None;
        colors.upper_window_border_sides = PaneSides::all(BorderStyle::None);

        let area = Rect::new(0, 0, 9, 3);
        let mut buf = Buffer::empty(area);
        let used = crate::render::upper_window::draw_upper_window(grid, false, &colors, area, &mut buf, true, &mut Vec::new());
        // SQ-0286: the Z-machine upper window's border preference is Unspecified, so
        // a theme with every border side disabled renders frameless (the theme
        // decides). The painted cells reproduce exactly at row 0.
        assert_eq!(used, 1, "one active upper row consumed, no frame");
        // cols=3 centered in 9 → x_off = 3.
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "A");
        assert_eq!(buf.cell((5, 0)).unwrap().symbol(), "Z");
    }

    // ── Transcript wrap cache (SQ-0305) ───────────────────────────────────────
    //
    // The cache holds the fully wrapped rows so an unchanged transcript is not
    // re-wrapped. These tests POISON the cached rows with a sentinel after a
    // render, then render again: a cache HIT leaves the sentinel in place (no
    // rebuild), while an invalidation rebuilds it from the real transcript
    // (sentinel gone). This directly observes hit vs. rebuild without a counter.

    fn wrap_render(state: &AppState, area: Rect) {
        let machine = minimal_machine();
        let status = crate::session::status_model_from_machine(&machine);
        let mut buf = Buffer::empty(area);
        render_transcript(&status, None, state, area, &mut buf, None);
    }

    fn poison_wrap_cache(state: &AppState) {
        let mut c = state.transcript_wrap.borrow_mut();
        let e = c.as_mut().expect("cache built by a prior render");
        e.rows.clear();
        e.rows.push(WrappedRow {
            text: "SENTINEL".to_string(),
            kind: TranscriptKind::Story,
            style: Style::default(),
            runs: Vec::new(),
            band: None,
            float: None,
        });
    }

    fn cached_first_text(state: &AppState) -> String {
        state
            .transcript_wrap
            .borrow()
            .as_ref()
            .expect("cache present")
            .rows
            .first()
            .map(|r| r.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn wrap_cache_hit_reuses_rows_when_nothing_changed() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        // Nothing changed → the second render must reuse the cached rows.
        wrap_render(&state, area);
        assert_eq!(cached_first_text(&state), "SENTINEL", "unchanged transcript must not re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_append() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.push_transcript_kind("more", TranscriptKind::Story); // bumps gen + len
        wrap_render(&state, area);
        assert_eq!(cached_first_text(&state), "hello world", "append must rebuild from real content");
    }

    #[test]
    fn wrap_cache_invalidates_on_width_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        wrap_render(&state, Rect::new(0, 0, 20, 8));
        poison_wrap_cache(&state);
        wrap_render(&state, Rect::new(0, 0, 30, 8)); // different wrap width
        assert_ne!(cached_first_text(&state), "SENTINEL", "width change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_filter_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.transcript_filter = TranscriptFilter::Meta; // hides the Story line
        wrap_render(&state, area);
        assert_ne!(cached_first_text(&state), "SENTINEL", "filter change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_anchor_change() {
        let mut state = AppState::default();
        state.push_transcript_kind("hello world", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        state.clear_anchor = Some(0); // screen-clear boundary moved
        wrap_render(&state, area);
        assert_ne!(cached_first_text(&state), "SENTINEL", "anchor change must re-wrap");
    }

    #[test]
    fn wrap_cache_invalidates_on_same_length_content_replacement() {
        // A rewind/restore can replace the transcript with a DIFFERENT content of
        // the SAME length — a length check alone would serve stale rows. The
        // generation counter (bumped by reset_transcript_sidecars) catches it.
        let mut state = AppState::default();
        state.push_transcript_kind("AAAAA", TranscriptKind::Story);
        let area = Rect::new(0, 0, 20, 8);
        wrap_render(&state, area);
        poison_wrap_cache(&state);
        let gen_before = state.transcript_gen;
        state.transcript = vec!["BBBBB".to_string()];
        state.transcript_kinds = vec![TranscriptKind::Story];
        state.transcript_runs = vec![Vec::new()];
        state.reset_transcript_sidecars(); // bumps gen; length unchanged (1)
        assert_ne!(state.transcript_gen, gen_before, "reset must bump the generation");
        assert_eq!(state.transcript.len(), 1, "same length as before");
        wrap_render(&state, area);
        assert_eq!(cached_first_text(&state), "BBBBB", "same-length replacement must re-wrap to new content");
    }

    #[test]
    fn window_wrapped_rows_windows_and_top_anchors() {
        let rows: Vec<WrappedRow> = (0..6)
            .map(|i| WrappedRow {
                text: format!("R{i}"),
                kind: TranscriptKind::Story,
                style: Style::default(),
                runs: Vec::new(),
                band: None,
                float: None,
            })
            .collect();
        // No anchor, scroll 0 → newest 3 at the bottom.
        let (vis, total, first) = window_wrapped_rows(&rows, None, 3, 0);
        assert_eq!(total, 6);
        assert_eq!(first, 3);
        assert_eq!(vis.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["R3", "R4", "R5"]);
        // Anchor at row 4 with the post-anchor content (2 rows) fitting the 3-row
        // viewport → pin from the anchor (top-anchored), fewer than `rows` returned.
        let (vis2, _t, first2) = window_wrapped_rows(&rows, Some(4), 3, 0);
        assert_eq!(first2, 4);
        assert_eq!(vis2.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["R4", "R5"]);
        // While scrolled (scroll != 0) the anchor does not apply.
        let (_v3, _t3, first3) = window_wrapped_rows(&rows, Some(4), 3, 1);
        assert_eq!(first3, 2);
    }
}
