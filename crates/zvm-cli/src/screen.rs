//! Basic DOS-style screen model for zvm-cli: pure formatting/SGR/terminal
//! helpers (this module) plus the stateful `ScreenView` (Task 3).

use zvm::io::TextAttrs;
use zvm::screen::{StatusLine, StatusRight, UpperWindow, ZColour, grey_rgb, rgb15_to_888};

pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

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
        ZColour::True24(v) => {
            let (r, g, b) = (((v >> 16) & 0xFF), ((v >> 8) & 0xFF), (v & 0xFF));
            params.push(format!("{};2;{};{};{}", base_true, r, g, b));
        }
    }
}

/// Resolve a `ZColour` to a 24-bit RGB triple for painting the terminal's
/// *page* background (OSC 11), mirroring `push_colour_sgr`'s per-variant
/// colour sources. `Default` (no game colour) is `None`. `Standard(2..=9)`
/// has no RGB source here either — `push_colour_sgr` emits a bare ANSI base
/// code for it and lets the terminal's own colour scheme resolve the actual
/// colour, so there is no concrete triple to paint OSC 11 with; that case is
/// also `None` rather than an invented palette value.
pub fn zcolour_rgb(c: ZColour) -> Option<(u8, u8, u8)> {
    match c {
        ZColour::Default => None,
        ZColour::Standard(2..=9) => None,
        ZColour::Standard(n) => Some(grey_rgb(n)),
        ZColour::True(v) => Some(rgb15_to_888(v)),
        ZColour::True24(v) => Some((((v >> 16) & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8)),
    }
}

// The page-background OSC and cursor-shape escapes live in `cli_host::term` —
// they were byte-identical here and in gvm-cli (SQ-0605).

/// SGR set-sequence (`ESC[...m`, no trailing reset) for `attrs`, or `""` when
/// no style/colour is active. Shared by `style_wrap` and the raw-mode input
/// editor (to echo typed input in the game's current style/colour).
pub fn sgr_open(attrs: TextAttrs) -> String {
    let mut params: Vec<String> = Vec::new();
    if attrs.style & 0x01 != 0 { params.push("7".into()); }
    if attrs.style & 0x02 != 0 { params.push("1".into()); }
    if attrs.style & 0x04 != 0 { params.push("3".into()); }
    push_colour_sgr(&mut params, attrs.fg, true);
    push_colour_sgr(&mut params, attrs.bg, false);
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

/// SGR that sets only the background colour (`ESC[..m`, no reset), or `""` when
/// the background is Default or colours are not honoured. Used to paint clears
/// and line padding with the game's chosen background.
pub fn bg_sgr(bg: ZColour, honor: bool) -> String {
    if !honor {
        return String::new();
    }
    let mut params: Vec<String> = Vec::new();
    push_colour_sgr(&mut params, bg, false);
    if params.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", params.join(";"))
    }
}

/// Wrap lower-window text in SGR when on a TTY and any style/colour is active; else plain.
pub fn style_wrap(s: &str, attrs: TextAttrs, is_tty: bool) -> String {
    if !is_tty {
        return s.to_string();
    }
    let open = sgr_open(attrs);
    if open.is_empty() {
        s.to_string()
    } else {
        format!("{}{}\x1b[0m", open, s)
    }
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
/// Carries style AND colour (fg/bg); colour is suppressed when `honor` is false.
pub fn upper_row_ansi(
    upper: &UpperWindow,
    row: u16,
    honor: bool,
    current_fg: ZColour,
    current_bg: ZColour,
) -> String {
    // Last column with any non-default attribute (blank cell = ' ' at style 0,
    // Default/Default); trailing defaults are dropped so the row closes reset,
    // matching the `ESC[2K` line-clear done before each row is written.
    let last = (1..=upper.cols)
        .rev()
        .find(|&c| {
            let cell = upper.cell(row, c);
            cell.ch != ' '
                || cell.style != 0
                || (honor && !matches!(cell.fg, ZColour::Default))
                || (honor && !matches!(cell.bg, ZColour::Default))
        })
        .unwrap_or(0);
    let mut out = String::new();
    let mut cur: Option<(u8, ZColour, ZColour)> = None;
    for c in 1..=last {
        let cell = upper.cell(row, c);
        let (style, fg, bg) = if honor {
            // The upper window's "default" colour is the screen's current
            // colour (what the row-fill paints); resolve Default cells to it so
            // blank/leading cells match the painted background instead of
            // resetting to terminal-default.
            let fg = if matches!(cell.fg, ZColour::Default) { current_fg } else { cell.fg };
            let bg = if matches!(cell.bg, ZColour::Default) { current_bg } else { cell.bg };
            (cell.style, fg, bg)
        } else {
            (cell.style, ZColour::Default, ZColour::Default)
        };
        if cur != Some((style, fg, bg)) {
            out.push_str("\x1b[0m");
            out.push_str(&sgr_open(TextAttrs { style, fg, bg }));
            cur = Some((style, fg, bg));
        }
        out.push(cell.ch);
    }
    if cur.is_some() {
        out.push_str("\x1b[0m");
    }
    out
}

/// Set the scroll region below the pinned rows, preserving the cursor position.
///
/// DECSTBM (`ESC[t;br`) homes the cursor as a side effect, which would
/// disconnect input from where the game left its `>` prompt. Save the cursor
/// (DECSC) before setting the region and restore it (DECRC) after, so input
/// happens at the prompt rather than being force-parked at the bottom row.
pub fn enter_region(top_rows: u16, term_rows: u16) -> String {
    format!("\x1b7\x1b[{};{}r\x1b8", top_rows + 1, term_rows)
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
    // Set by `erase`; consumed by the next `render`. When set, a subsequent
    // upper-window growth shifts the freshly-streamed lower prompt down below the
    // new upper region (see the shift logic in `render`). Only meaningful right
    // after an `erase_window`, so continuous flow (no erase) is never shifted.
    pending_erase_shift: bool,
}

impl ScreenView {
    pub fn new(is_tty: bool, no_status: bool, term_rows: u16) -> Self {
        ScreenView {
            is_tty,
            no_status,
            term_rows,
            active_rows: 0,
            last_block: None,
            pending_erase_shift: false,
        }
    }

    /// Number of pinned top rows for the current machine state.
    fn top_rows(machine: &Machine) -> u16 {
        if machine.mem.version() < 4 {
            1 // v1-v3: a status line is always shown
        } else {
            // The grid's own row count: equals `upper_window_rows` normally, but
            // may be larger when a game drew in the upper window below the split
            // (e.g. LostPig's HELP menu). Render every row the game wrote.
            machine.screen.upper.rows
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
            (1..=top)
                .map(|r| {
                    upper_row_ansi(
                        &machine.screen.upper,
                        r,
                        machine.honor_game_colours,
                        machine.screen.current_fg,
                        machine.screen.current_bg,
                    )
                })
                .collect()
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
        let bg_paint = bg_sgr(machine.screen.current_bg, machine.honor_game_colours);
        self.render(top, &plain, &ansi, &bg_paint)
    }

    /// Pure-core renderer: given the pinned-row count and the already-formatted
    /// plain/ANSI rows, advance the view's state and return the bytes to write.
    fn render(&mut self, top: u16, rows_plain: &[String], rows_ansi: &[String], bg_paint: &str) -> String {
        if self.no_status {
            return String::new();
        }
        if self.is_tty {
            let mut out = String::new();
            // The shift below is a one-shot consumed on the first frame after an
            // erase; read and clear it now.
            let erase_shift = self.pending_erase_shift;
            self.pending_erase_shift = false;
            if top != self.active_rows {
                let delta = top as i32 - self.active_rows as i32;
                if erase_shift && delta > 0 && top >= 2 {
                    // A game cleared the screen, then redrew a multi-row upper
                    // window (e.g. BeyondZork's stats + compass) and streamed its
                    // short lower-window prompt right after the clear — before
                    // this frame pins the upper region. That prompt (and the
                    // input cursor) would otherwise sit inside the soon-to-be-
                    // painted upper region, on top of the compass. Push the lower
                    // window — its streamed text AND the cursor — down by the
                    // delta so the cursor lands in the lower window and the prompt
                    // stays visible (ZMSD §8.7.2.2).
                    //
                    // Sequence: DECSC to capture the real prompt cursor, reset to
                    // full screen, scroll the display down (SD), DECRC to restore
                    // the cursor, then a relative cursor-down (CUD) by the delta.
                    // Wrapping SD in DECSC/DECRC normalises the terminal's SD
                    // cursor side-effect, which is NOT portable (tmux homes the
                    // cursor on SD; xterm leaves it put) — so the relative CUD then
                    // lands on the shifted prompt with its real column intact.
                    //
                    // Gated on BOTH the just-erased flag and a multi-row upper
                    // window (top >= 2). In continuous flow (no erase) the content
                    // is already positioned correctly. And a v3 game also erases
                    // at startup but then streams a full screen of narrative under
                    // a 1-row status line (top == 1) — scrolling that would garble
                    // it — so the 1-row status case is excluded.
                    out.push_str("\x1b7"); // DECSC: save real prompt cursor
                    out.push_str(&leave_region());
                    out.push_str(&format!("\x1b[{delta}T")); // SD: content down
                    out.push_str("\x1b8"); // DECRC: restore prompt cursor
                    out.push_str(&format!("\x1b[{delta}B")); // CUD: follow content
                    out.push_str(&enter_region(top, self.term_rows));
                } else {
                    out.push_str(&if top == 0 {
                        leave_region()
                    } else {
                        enter_region(top, self.term_rows)
                    });
                }
                self.active_rows = top;
            }
            if top > 0 {
                out.push_str("\x1b7"); // DECSC save cursor
                for (i, row) in rows_ansi.iter().enumerate() {
                    // position, paint bg, clear-to-EOL (fills with bg), then row
                    out.push_str(&format!("\x1b[{};1H{}\x1b[2K", i as u16 + 1, bg_paint));
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
    pub fn erase(&mut self, bg: ZColour, honor: bool) -> String {
        if !self.is_tty {
            return String::new();
        }
        self.active_rows = 0;
        // Arm the one-shot lower-window shift: if the game now redraws a
        // multi-row upper window, the next `render` pushes the freshly-streamed
        // lower prompt down below it (see `render`).
        self.pending_erase_shift = true;
        let paint = bg_sgr(bg, honor);
        if paint.is_empty() {
            format!("{}\x1b[2J\x1b[H", leave_region())
        } else {
            // Set bg, clear (fills with bg), home, then reset so subsequent
            // prompt/text is not forced onto the painted background run.
            format!("{}{}\x1b[2J\x1b[H\x1b[0m", leave_region(), paint)
        }
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
        assert_eq!(piped.render(1, &p, &a, ""), "");
        let mut tty = ScreenView::new(true, true, 24);
        assert_eq!(tty.render(1, &p, &a, ""), "");
    }

    #[test]
    fn piped_emits_inline_block_once_then_dedupes() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(false, false, 24);
        let first = v.render(1, &p, &a, "");
        assert!(first.contains("West of House"), "first frame emits block: {first:?}");
        assert!(!first.contains('\x1b'), "inline block carries no ANSI: {first:?}");
        let second = v.render(1, &p, &a, "");
        assert_eq!(second, "", "unchanged region dedupes to empty");
    }

    #[test]
    fn tty_enters_region_then_resets_on_leave() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let f = v.render(1, &p, &a, "");
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
        let _ = v.render(1, &p, &a, ""); // activate a region (active_rows = 1)
        let out = v.erase(ZColour::Default, true);
        assert!(out.contains("\x1b[r"), "erase leaves the scroll region: {out:?}");
        assert!(out.contains("\x1b[2J"), "erase clears the screen: {out:?}");
        assert!(out.ends_with("\x1b[H"), "erase homes the cursor: {out:?}");
        // After erase the region is considered inactive, so the next frame
        // re-establishes it.
        let re = v.render(1, &p, &a, "");
        assert!(re.contains("\x1b[2;24r"), "next frame re-enters the region: {re:?}");
    }

    #[test]
    fn erase_is_noop_when_piped() {
        let mut v = ScreenView::new(false, false, 24);
        assert_eq!(v.erase(ZColour::Default, true), "", "piped erase emits nothing");
    }

    #[test]
    fn tty_dropping_to_zero_rows_resets_region() {
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let _ = v.render(1, &p, &a, ""); // activate region
        let out = v.render(0, &[], &[], "");
        assert!(out.contains("\x1b[r"), "dropping to 0 rows resets region: {out:?}");
    }

    #[test]
    fn erase_then_multirow_upper_shifts_lower_window() {
        // BeyondZork case: erase, then a game redraws a 12-row upper window with
        // a short lower prompt already streamed at the top. The next frame must
        // scroll the lower window down and follow the cursor below the region.
        let rows = vec![String::new(); 12];
        let mut v = ScreenView::new(true, false, 24);
        let _ = v.erase(ZColour::Default, true); // arms the one-shot shift
        let out = v.render(12, &rows, &rows, "");
        assert!(out.contains("\x1b[12T"), "scrolls the display down by 12 (SD): {out:?}");
        assert!(out.contains("\x1b[12B"), "follows the cursor down by 12 (CUD): {out:?}");
        assert!(out.contains("\x1b[13;24r"), "pins the region below the upper window: {out:?}");
        // SD is wrapped in DECSC/DECRC so the terminal's SD cursor side-effect is
        // normalised before the relative CUD.
        assert!(
            out.contains("\x1b7\x1b[r\x1b[12T\x1b8\x1b[12B"),
            "SD is wrapped in DECSC/DECRC, then CUD: {out:?}"
        );
        // One-shot: a subsequent height change does NOT shift again.
        let again = v.render(6, &rows[..6], &rows[..6], "");
        assert!(!again.contains("\x1b[6T") && !again.contains("\x1b[6S"), "shift is one-shot: {again:?}");
    }

    #[test]
    fn erase_then_single_row_status_does_not_shift() {
        // A v3 game also erases at startup, but its 1-row status line (top == 1)
        // must NOT trigger a shift — it streams a full screen of narrative that
        // scrolling would garble.
        let (p, a) = v3_rows();
        let mut v = ScreenView::new(true, false, 24);
        let _ = v.erase(ZColour::Default, true);
        let out = v.render(1, &p, &a, "");
        assert!(!out.contains("\x1b[1T"), "1-row status line is not shifted: {out:?}");
        assert!(out.contains("\x1b[2;24r"), "still pins the status region: {out:?}");
    }

    #[test]
    fn multirow_upper_without_erase_does_not_shift() {
        // Continuous flow (no preceding erase): the content is already positioned
        // correctly, so a multi-row upper window must NOT scroll the lower window.
        let rows = vec![String::new(); 12];
        let mut v = ScreenView::new(true, false, 24);
        let out = v.render(12, &rows, &rows, "");
        assert!(!out.contains("\x1b[12T"), "no erase means no shift: {out:?}");
        assert!(out.contains("\x1b[13;24r"), "still pins the region: {out:?}");
    }
}

#[cfg(test)]
mod colour_tests {
    use super::*;
    use zvm::io::TextAttrs;
    use zvm::screen::ZColour;

    #[test]
    fn sgr_open_builds_prefix_without_reset() {
        assert_eq!(sgr_open(TextAttrs::default()), "", "no attrs → empty");
        assert_eq!(sgr_open(TextAttrs { style: 2, ..Default::default() }), "\x1b[1m", "bold, no reset");
        let c = TextAttrs { style: 0, fg: ZColour::Standard(3), bg: ZColour::Default };
        assert_eq!(sgr_open(c), "\x1b[31m", "fg only, no trailing reset");
        // style_wrap composes sgr_open + reset.
        assert_eq!(style_wrap("x", c, true), "\x1b[31mx\x1b[0m");
    }

    #[test]
    fn bg_sgr_sets_background_only() {
        use zvm::screen::ZColour;
        assert_eq!(bg_sgr(ZColour::Standard(2), true), "\x1b[40m", "black bg");
        assert_eq!(bg_sgr(ZColour::Default, true), "", "default = no SGR");
        assert_eq!(bg_sgr(ZColour::Standard(2), false), "", "honor off = no SGR");
    }

    #[test]
    fn erase_paints_current_bg() {
        use zvm::screen::ZColour;
        let mut v = ScreenView::new(true, false, 24);
        let out = v.erase(ZColour::Standard(2), true);
        assert!(out.contains("\x1b[40m"), "bg SGR before clear: {out:?}");
        assert!(out.contains("\x1b[2J"), "screen clear present: {out:?}");
        assert!(out.find("\x1b[40m").unwrap() < out.find("\x1b[2J").unwrap(),
            "bg set before the clear: {out:?}");
    }

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
        // grey 11 -> 8C8C8C (ZMSD §8.3.1: medium grey is true colour $4631;
        // this used to pin the invented #808080)
        let g = TextAttrs { style: 0, fg: ZColour::Standard(11), bg: ZColour::Default };
        assert_eq!(style_wrap("x", g, true), "\x1b[38;2;140;140;140mx\x1b[0m");
        // non-tty stays plain
        assert_eq!(style_wrap("x", a, false), "x");
    }
}

#[cfg(test)]
mod page_bg_tests {
    use zvm::screen::ZColour;

    // The OSC escapes and their change-detection are covered in
    // `cli_host::term` now; what stays here is the Z-machine-specific question
    // of which ZColours have an RGB triple to paint with at all.

    #[test]
    fn zcolour_rgb_default_is_none_true24_unpacks() {
        assert_eq!(super::zcolour_rgb(ZColour::Default), None);
        assert_eq!(super::zcolour_rgb(ZColour::True24(0x123456)), Some((0x12, 0x34, 0x56)));
    }

    #[test]
    fn zcolour_rgb_standard_and_true() {
        // 2..=9 are scheme-relative in push_colour_sgr (bare ANSI code, no RGB
        // source) — no invented palette here either.
        assert_eq!(super::zcolour_rgb(ZColour::Standard(3)), None);
        // 10..=12 resolve via the shared grey_rgb table, same as push_colour_sgr.
        // ZMSD §8.3.1 fixes medium grey (11) at true colour $4631 → #8C8C8C.
        // (This assertion previously pinned the invented #808080.)
        assert_eq!(super::zcolour_rgb(ZColour::Standard(11)), Some((0x8C, 0x8C, 0x8C)));
        // True colour goes through rgb15_to_888, same as push_colour_sgr.
        assert_eq!(super::zcolour_rgb(ZColour::True(0x001F)), Some((255, 0, 0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zvm::screen::{StatusLine, StatusRight, UpperWindow, ZColour};

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
        let ansi = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Default);
        assert!(
            ansi.contains("\x1b[1m") && ansi.ends_with("\x1b[0m"),
            "ansi: {ansi:?}"
        );
    }

    #[test]
    fn upper_row_ansi_emits_per_cell_fg_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // "Hi" in red-on-black, honor on.
        u.put(1, 1, 'H', 0, ZColour::Standard(3), ZColour::Standard(2));
        u.put(1, 2, 'i', 0, ZColour::Standard(3), ZColour::Standard(2));
        let on = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Default);
        assert!(on.contains("31"), "red fg SGR present: {on:?}");
        assert!(on.contains("40"), "black bg SGR present: {on:?}");
        assert!(on.contains("Hi"), "text present: {on:?}");
        // honor off: no colour SGR, text still present.
        let off = upper_row_ansi(&u, 1, false, ZColour::Default, ZColour::Default);
        assert!(!off.contains("31") && !off.contains("40"), "no colour when honor off: {off:?}");
        assert!(off.contains("Hi"), "text present when honor off: {off:?}");
    }

    #[test]
    fn upper_row_ansi_paints_default_cells_with_current_bg() {
        use zvm::screen::ZColour;
        let mut u = UpperWindow::default();
        u.resize(1, 6);
        // cols 1-2 blank (Default bg); cols 3-4 'Hi' explicit white-on-black.
        u.put(1, 3, 'H', 0, ZColour::Standard(9), ZColour::Standard(2));
        u.put(1, 4, 'i', 0, ZColour::Standard(9), ZColour::Standard(2));
        // Screen background is black: leading blank cells must be painted black
        // (bg 40) BEFORE the text, not left to reset-to-terminal-default.
        let out = upper_row_ansi(&u, 1, true, ZColour::Default, ZColour::Standard(2));
        // The text run at col 3 emits its own bg 40, so a plain "40 before H"
        // check would pass even unfixed. Discriminate: the segment BEFORE the
        // first (leading) space must already carry bg 40, i.e. the leading
        // blank cells are painted with the screen background rather than a bare
        // reset. Unfixed, the leading run is a lone `ESC[0m` with no 40.
        let first_space = out.find(' ').expect("row has leading spaces");
        assert!(
            out[..first_space].contains("40"),
            "leading blank cells carry the screen bg before the spaces: {out:?}"
        );
        assert!(out.contains('H'), "text preserved: {out:?}");
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
        // Sets the scroll region (rows 2..24) wrapped in DECSC/DECRC so the
        // cursor is preserved (not parked at the bottom).
        assert_eq!(enter_region(1, 24), "\x1b7\x1b[2;24r\x1b8");
        assert!(!enter_region(1, 24).contains("\x1b[24;1H"), "no bottom-row park");
    }
}
