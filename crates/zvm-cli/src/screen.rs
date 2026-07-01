//! Basic DOS-style screen model for zvm-cli: pure formatting/SGR/terminal
//! helpers (this module) plus the stateful `ScreenView` (Task 3).

use zvm::io::TextAttrs;
use zvm::screen::{StatusLine, StatusRight, UpperWindow, ZColour, grey_rgb, rgb15_to_888};

pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

/// SGR set-codes for a Z-machine text-style bitmask (no leading/trailing reset).
/// 1=reverse, 2=bold, 4=italic; 8 (fixed-pitch) has no terminal equivalent here.
pub fn sgr_set(style: u8) -> String {
    let mut s = String::new();
    if style & 0x01 != 0 {
        s.push_str("\x1b[7m");
    }
    if style & 0x02 != 0 {
        s.push_str("\x1b[1m");
    }
    if style & 0x04 != 0 {
        s.push_str("\x1b[3m");
    }
    s
}

/// Push SGR parameters for one colour channel. `fg` selects 3x vs 4x codes.
fn push_colour_sgr(params: &mut Vec<String>, c: ZColour, fg: bool) {
    let (base_std, base_true) = if fg { (30u16, 38u16) } else { (40u16, 48u16) };
    match c {
        ZColour::Default => {}
        ZColour::Standard(n @ 2..=9) => params.push((base_std + (n as u16 - 2)).to_string()),
        ZColour::Standard(n) => {
            let (r, g, b) = grey_rgb(n);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
        ZColour::True(v) => {
            let (r, g, b) = rgb15_to_888(v);
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
    }
}

/// Wrap lower-window text in SGR when on a TTY and any style/colour is active; else plain.
pub fn style_wrap(s: &str, attrs: TextAttrs, is_tty: bool) -> String {
    if !is_tty {
        return s.to_string();
    }
    let mut params: Vec<String> = Vec::new();
    if attrs.style & 0x01 != 0 { params.push("7".into()); }
    if attrs.style & 0x02 != 0 { params.push("1".into()); }
    if attrs.style & 0x04 != 0 { params.push("3".into()); }
    push_colour_sgr(&mut params, attrs.fg, true);
    push_colour_sgr(&mut params, attrs.bg, false);
    if params.is_empty() {
        return s.to_string();
    }
    format!("\x1b[{}m{}\x1b[0m", params.join(";"), s)
}

/// Terminal BEL per bleep, TTY-gated.
pub fn bleep_bytes(count: usize, is_tty: bool) -> String {
    if is_tty {
        "\x07".repeat(count)
    } else {
        String::new()
    }
}

/// True once `lines` reaches the page limit (`page_height - 1`); a height < 2
/// never pages (avoids a zero/looping page).
pub fn should_page(lines: u16, page_height: u16) -> bool {
    page_height >= 2 && lines >= page_height - 1
}

fn right_field(right: &StatusRight) -> String {
    match right {
        StatusRight::ScoreTurns { score, turns } => format!("Score: {score}  Moves: {turns}"),
        StatusRight::Time { hours, minutes } => format!("Time: {hours:02}:{minutes:02}"),
    }
}

/// Plain v3 status row: " `location` ... `right` ", padded to exactly `cols`.
pub fn status_text(st: &StatusLine, cols: u16) -> String {
    let cols = cols as usize;
    if cols < 2 {
        return " ".repeat(cols);
    }
    let inner = cols - 2; // one border space each side
    let right = right_field(&st.right);
    let right_w = right.chars().count().min(inner);
    let right: String = right.chars().take(right_w).collect();
    let left_max = inner - right_w;
    let left: String = st.location.chars().take(left_max).collect();
    let fill = inner - left.chars().count() - right_w;
    format!(" {}{}{} ", left, " ".repeat(fill), right)
}

/// Plain text of one upper-window row, trailing blanks trimmed.
pub fn upper_row_text(upper: &UpperWindow, row: u16) -> String {
    let mut s = String::new();
    for c in 1..=upper.cols {
        s.push(upper.cell(row, c).ch);
    }
    s.trim_end().to_string()
}

/// One upper-window row with per-cell SGR runs (for the pinned TTY region).
pub fn upper_row_ansi(upper: &UpperWindow, row: u16) -> String {
    // Last column with non-blank content (a blank cell is ' ' at style 0);
    // trailing blanks are dropped so the row closes with a reset, matching the
    // line-clear (`ESC[2K`) done before each row is written in the TTY region.
    let last = (1..=upper.cols)
        .rev()
        .find(|&c| {
            let cell = upper.cell(row, c);
            cell.ch != ' ' || cell.style != 0
        })
        .unwrap_or(0);
    let mut out = String::new();
    let mut cur = 0u8;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        if cell.style != cur {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_set(cell.style));
            cur = cell.style;
        }
        out.push(cell.ch);
    }
    if cur != 0 {
        out.push_str("\x1b[0m");
    }
    out
}

/// Set the scroll region below the pinned rows and park the cursor at the
/// bottom of the lower region.
pub fn enter_region(top_rows: u16, term_rows: u16) -> String {
    format!(
        "\x1b[{};{}r\x1b[{};1H",
        top_rows + 1,
        term_rows,
        term_rows
    )
}

/// Reset the scroll region to the full screen.
pub fn leave_region() -> String {
    "\x1b[r".to_string()
}

// ---------------------------------------------------------------------------
// ScreenView — stateful top-region rendering
// ---------------------------------------------------------------------------

use zvm::cpu::exec::Machine;

/// Tracks the pinned top-region state and produces the bytes to emit before an
/// input prompt: an ANSI scroll-region update on a TTY, or a deduped inline
/// plain-text block when piped.
pub struct ScreenView {
    is_tty: bool,
    no_status: bool,
    term_rows: u16,
    active_rows: u16,           // current scroll-region top height (TTY)
    last_block: Option<String>, // last inline block emitted (non-TTY dedupe)
}

impl ScreenView {
    pub fn new(is_tty: bool, no_status: bool, term_rows: u16) -> Self {
        ScreenView { is_tty, no_status, term_rows, active_rows: 0, last_block: None }
    }

    /// Number of pinned top rows for the current machine state.
    fn top_rows(machine: &Machine) -> u16 {
        if machine.mem.version() < 4 {
            1 // v1-v3: a status line is always shown
        } else {
            machine.screen.upper_window_rows
        }
    }

    /// Plain-text rows of the top region (status row for v3, the upper grid for
    /// v4+). Empty vec when there is no region.
    fn rows_plain(machine: &Machine, top: u16) -> Vec<String> {
        if top == 0 {
            return Vec::new();
        }
        if machine.mem.version() < 4 {
            vec![status_text(&machine.status_line(), DEFAULT_COLS)]
        } else {
            (1..=top).map(|r| upper_row_text(&machine.screen.upper, r)).collect()
        }
    }

    /// ANSI rows of the top region (reverse-video bar for v3, per-cell SGR runs
    /// for v4+).
    fn rows_ansi(machine: &Machine, top: u16) -> Vec<String> {
        if top == 0 {
            return Vec::new();
        }
        if machine.mem.version() < 4 {
            vec![format!("\x1b[7m{}\x1b[0m", status_text(&machine.status_line(), DEFAULT_COLS))]
        } else {
            (1..=top).map(|r| upper_row_ansi(&machine.screen.upper, r)).collect()
        }
    }

    /// Bytes to emit just before an input prompt.
    pub fn frame(&mut self, machine: &Machine) -> String {
        if self.no_status {
            return String::new();
        }
        let top = Self::top_rows(machine);
        let plain = Self::rows_plain(machine, top);
        let ansi = Self::rows_ansi(machine, top);
        self.render(top, &plain, &ansi)
    }

    /// Pure-core renderer: given the pinned-row count and the already-formatted
    /// plain/ANSI rows, advance the view's state and return the bytes to write.
    fn render(&mut self, top: u16, rows_plain: &[String], rows_ansi: &[String]) -> String {
        if self.no_status {
            return String::new();
        }
        if self.is_tty {
            let mut out = String::new();
            if top != self.active_rows {
                out.push_str(&if top == 0 {
                    leave_region()
                } else {
                    enter_region(top, self.term_rows)
                });
                self.active_rows = top;
            }
            if top > 0 {
                out.push_str("\x1b7"); // DECSC save cursor
                for (i, row) in rows_ansi.iter().enumerate() {
                    out.push_str(&format!("\x1b[{};1H\x1b[2K", i as u16 + 1)); // row, clear
                    out.push_str(row);
                }
                out.push_str("\x1b8"); // DECRC restore cursor
            }
            out
        } else {
            if top == 0 {
                return String::new();
            }
            let block = {
                let mut rows: Vec<String> =
                    rows_plain.iter().map(|r| r.trim_end().to_string()).collect();
                while rows.last().map(|r| r.is_empty()).unwrap_or(false) {
                    rows.pop();
                }
                if rows.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", rows.join("\n"))
                }
            };
            if block.is_empty() || self.last_block.as_deref() == Some(block.as_str()) {
                return String::new();
            }
            self.last_block = Some(block.clone());
            block
        }
    }

    /// Update the row count used for scroll-region sizing (call on terminal resize).
    pub fn set_term_rows(&mut self, rows: u16) {
        self.term_rows = rows;
    }

    /// Clear+home the screen at startup (interactive only), so existing
    /// scrollback is not overwritten by the pinned region.
    pub fn start(&self) -> String {
        if self.is_tty && !self.no_status {
            "\x1b[2J\x1b[H".to_string()
        } else {
            String::new()
        }
    }

    /// Bytes to clear the lower window in response to an `erase_window`
    /// request (ZMSD §8.7.3): leave any scroll region, clear the whole screen,
    /// home the cursor, and reset the pinned-region state so the next `frame`
    /// re-establishes the region. On a piped/non-TTY sink there is no screen to
    /// clear (streaming scrollback), so it is a no-op.
    pub fn erase(&mut self) -> String {
        if !self.is_tty {
            return String::new();
        }
        self.active_rows = 0;
        format!("{}\x1b[2J\x1b[H", leave_region())
    }

    /// Restore the terminal at quit.
    pub fn leave(&mut self) -> String {
        if self.is_tty && self.active_rows > 0 {
            self.active_rows = 0;
            format!("{}\x1b[{};1H", leave_region(), self.term_rows)
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight};

    /// A hand-built v3 status region (plain + reverse-video ANSI rows).
    fn v3_rows() -> (Vec<String>, Vec<String>) {
        let st = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 0, turns: 1 },
        };
        let plain = vec![status_text(&st, DEFAULT_COLS)];
        let ansi = vec![format!("\x1b[7m{}\x1b[0m", status_text(&st, DEFAULT_COLS))];
        (plain, ansi)
    }

    #[test]
    fn no_status_suppresses_everything() {
        let (p, a) = v3_rows();
        let mut piped = ScreenView::new(false, true, 24);
        assert_eq!(piped.render(1, &p, &a), "");
        let mut tty = ScreenView::new(true, true, 24);
        assert_eq!(tty.render(1, &p, &a), "");
    }

    #[test]
    fn piped_emits_inline_block_once_then_dedupes() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(false, false, 24);
        let first = v.render(1, &p, &a);
        assert!(first.contains("West of House"), "first frame emits block: {first:?}");
        assert!(!first.contains('\x1b'), "inline block carries no ANSI: {first:?}");
        let second = v.render(1, &p, &a);
        assert_eq!(second, "", "unchanged region dedupes to empty");
    }

    #[test]
    fn tty_enters_region_then_resets_on_leave() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let f = v.render(1, &p, &a);
        assert!(f.contains("\x1b[2;24r"), "sets scroll region: {f:?}");
        assert!(f.contains("\x1b[7m"), "v3 status bar is reverse-video: {f:?}");
        assert!(v.leave().contains("\x1b[r"), "leave resets region");
    }

    #[test]
    fn start_clears_screen_only_when_interactive() {
        assert_eq!(ScreenView::new(true, false, 24).start(), "\x1b[2J\x1b[H");
        assert_eq!(ScreenView::new(false, false, 24).start(), ""); // piped
        assert_eq!(ScreenView::new(true, true, 24).start(), ""); // --no-status
    }

    #[test]
    fn erase_clears_screen_and_resets_region_on_tty() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let _ = v.render(1, &p, &a); // activate a region (active_rows = 1)
        let out = v.erase();
        assert!(out.contains("\x1b[r"), "erase leaves the scroll region: {out:?}");
        assert!(out.contains("\x1b[2J"), "erase clears the screen: {out:?}");
        assert!(out.ends_with("\x1b[H"), "erase homes the cursor: {out:?}");
        // After erase the region is considered inactive, so the next frame
        // re-establishes it.
        let re = v.render(1, &p, &a);
        assert!(re.contains("\x1b[2;24r"), "next frame re-enters the region: {re:?}");
    }

    #[test]
    fn erase_is_noop_when_piped() {
        let mut v = ScreenView::new(false, false, 24);
        assert_eq!(v.erase(), "", "piped erase emits nothing");
    }

    #[test]
    fn tty_dropping_to_zero_rows_resets_region() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let _ = v.render(1, &p, &a); // activate region
        let out = v.render(0, &[], &[]);
        assert!(out.contains("\x1b[r"), "dropping to 0 rows resets region: {out:?}");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use zvm::io::TextAttrs;
    use zvm::screen::ZColour;

    #[test]
    fn style_wrap_emits_colour_sgr() {
        // standard fg=red(3)->31, bg=blue(6)->44
        let a = TextAttrs { style: 0, fg: ZColour::Standard(3), bg: ZColour::Standard(6) };
        assert_eq!(style_wrap("x", a, true), "\x1b[31;44mx\x1b[0m");
        // default channels emit nothing; no attrs → no wrap
        let d = TextAttrs { style: 0, fg: ZColour::Default, bg: ZColour::Default };
        assert_eq!(style_wrap("x", d, true), "x");
        // true colour fg
        let t = TextAttrs { style: 0, fg: ZColour::True(0x7FFF), bg: ZColour::Default };
        assert_eq!(style_wrap("x", t, true), "\x1b[38;2;255;255;255mx\x1b[0m");
        // grey 11 -> 808080
        let g = TextAttrs { style: 0, fg: ZColour::Standard(11), bg: ZColour::Default };
        assert_eq!(style_wrap("x", g, true), "\x1b[38;2;128;128;128mx\x1b[0m");
        // non-tty stays plain
        assert_eq!(style_wrap("x", a, false), "x");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight, UpperWindow, ZColour};

    #[test]
    fn sgr_set_maps_bits() {
        assert_eq!(sgr_set(0), "");
        assert_eq!(sgr_set(1), "\x1b[7m"); // reverse
        assert_eq!(sgr_set(2), "\x1b[1m"); // bold
        assert_eq!(sgr_set(4), "\x1b[3m"); // italic
        assert_eq!(sgr_set(8), ""); // fixed-pitch ignored
        assert_eq!(sgr_set(1 | 2), "\x1b[7m\x1b[1m");
    }

    #[test]
    fn style_wrap_only_when_tty_and_styled() {
        use zvm::io::TextAttrs;
        assert_eq!(style_wrap("hi", TextAttrs { style: 0, ..Default::default() }, true), "hi");
        assert_eq!(style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, false), "hi");
        assert_eq!(style_wrap("hi", TextAttrs { style: 2, ..Default::default() }, true), "\x1b[1mhi\x1b[0m");
    }

    #[test]
    fn bleep_bytes_tty_gated() {
        assert_eq!(bleep_bytes(3, true), "\x07\x07\x07");
        assert_eq!(bleep_bytes(3, false), "");
        assert_eq!(bleep_bytes(0, true), "");
    }

    #[test]
    fn status_text_pads_and_right_aligns() {
        let st = StatusLine {
            location: "West of House".into(),
            right: StatusRight::ScoreTurns { score: 0, turns: 1 },
        };
        let row = status_text(&st, 40);
        assert_eq!(row.chars().count(), 40, "padded to width");
        assert!(row.starts_with(" West of House"), "location left: {row:?}");
        assert!(row.trim_end().ends_with("Moves: 1"), "right field: {row:?}");
    }

    #[test]
    fn status_text_truncates_long_location() {
        let st = StatusLine {
            location: "x".repeat(100),
            right: StatusRight::Time { hours: 9, minutes: 5 },
        };
        let row = status_text(&st, 20);
        assert_eq!(row.chars().count(), 20);
        assert!(row.contains("09:05"));
    }

    #[test]
    fn upper_row_text_and_ansi() {
        let mut u = UpperWindow::default();
        u.resize(1, 5);
        u.put(1, 1, 'H', 0, ZColour::Default, ZColour::Default);
        u.put(1, 2, 'i', 2, ZColour::Default, ZColour::Default); // bold
        let text = upper_row_text(&u, 1);
        assert_eq!(text, "Hi"); // trailing blanks trimmed
        let ansi = upper_row_ansi(&u, 1);
        assert!(
            ansi.contains("\x1b[1m") && ansi.ends_with("\x1b[0m"),
            "ansi: {ansi:?}"
        );
    }

    #[test]
    fn should_page_at_threshold() {
        assert!(!should_page(0, 24));
        assert!(!should_page(22, 24));
        assert!(should_page(23, 24)); // page_height - 1
        assert!(should_page(99, 24));
        assert!(!should_page(5, 1)); // degenerate height never pages
    }

    #[test]
    fn region_strings() {
        assert_eq!(leave_region(), "\x1b[r");
        assert!(enter_region(1, 24).starts_with("\x1b[2;24r"));
    }
}
