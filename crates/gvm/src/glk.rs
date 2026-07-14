//! The Glk window/stream/output model — the interactive-fiction subset of Glk
//! (Andrew Plotkin's Glk spec 0.7.5), transcribed into `GLULX_NOTES.md` §19.
//!
//! The model ([`Model`]) owns the window tree, the streams, the current output
//! stream, and per-stream styles. It is pure bookkeeping: it never touches
//! Glulx main memory (memory-stream byte moves are done by the execution engine,
//! which holds both the model and the [`Memory`](crate::memory::Memory)) and it
//! never renders — a pluggable [`GlkBackend`] does the display. `@glk` selectors
//! in `exec.rs` operate on this model and drive the backend for output.
//!
//! All constant values (window types, split methods, style classes, gestalt
//! selectors, dispatch selector codes) are the values from `glk.h`, recorded in
//! `GLULX_NOTES.md` §19.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};

// ── Window types (`wintype_*`, the `wintype` argument to glk_window_open) ──────

/// The window kinds this subset supports. (Blank is out of scope.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WinType {
    /// Internal layout node created by a split (`wintype_Pair` = 1).
    Pair,
    /// Scrolling main text window (`wintype_TextBuffer` = 3).
    TextBuffer,
    /// Fixed character grid / status window (`wintype_TextGrid` = 4).
    TextGrid,
    /// Pixel-canvas graphics window (`wintype_Graphics` = 5).
    Graphics,
}

impl WinType {
    /// Map a `wintype` argument value to a supported type (None if unsupported).
    pub fn from_arg(v: u32) -> Option<WinType> {
        match v {
            1 => Some(WinType::Pair),
            3 => Some(WinType::TextBuffer),
            4 => Some(WinType::TextGrid),
            5 => Some(WinType::Graphics),
            _ => None,
        }
    }
    /// The `glk_window_get_type` value for this type.
    pub fn to_arg(self) -> u32 {
        match self {
            WinType::Pair => 1,
            WinType::TextBuffer => 3,
            WinType::TextGrid => 4,
            WinType::Graphics => 5,
        }
    }
}

// ── Split methods (`winmethod_*`) ─────────────────────────────────────────────

/// Mask selecting the split direction bits of a `winmethod`.
pub const WINMETHOD_DIRMASK: u32 = 0x0f;
/// New window to the left of the split window.
pub const WINMETHOD_LEFT: u32 = 0x00;
/// New window to the right of the split window.
pub const WINMETHOD_RIGHT: u32 = 0x01;
/// New window above the split window.
pub const WINMETHOD_ABOVE: u32 = 0x02;
/// New window below the split window.
pub const WINMETHOD_BELOW: u32 = 0x03;
/// Mask selecting the division (sizing) bits of a `winmethod`.
pub const WINMETHOD_DIVISIONMASK: u32 = 0xf0;
/// Fixed-size split: `size` is a character count.
pub const WINMETHOD_FIXED: u32 = 0x10;
/// Proportional split: `size` is a percentage (0–100) of the parent.
pub const WINMETHOD_PROPORTIONAL: u32 = 0x20;
/// Mask selecting the border bits of a `winmethod`.
pub const WINMETHOD_BORDERMASK: u32 = 0x0100;
/// Draw a border between the split's two children (the default; bit clear).
pub const WINMETHOD_BORDER: u32 = 0x0000;
/// Request NO border between the split's two children (a hint).
pub const WINMETHOD_NOBORDER: u32 = 0x0100;

// ── Style classes (`style_*`) ─────────────────────────────────────────────────

/// A Glk style class (the `style_*` constants 0–10).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlkStyle {
    /// `style_Normal` = 0.
    Normal,
    /// `style_Emphasized` = 1.
    Emphasized,
    /// `style_Preformatted` = 2.
    Preformatted,
    /// `style_Header` = 3.
    Header,
    /// `style_Subheader` = 4.
    Subheader,
    /// `style_Alert` = 5.
    Alert,
    /// `style_Note` = 6.
    Note,
    /// `style_BlockQuote` = 7.
    BlockQuote,
    /// `style_Input` = 8.
    Input,
    /// `style_User1` = 9.
    User1,
    /// `style_User2` = 10.
    User2,
}

/// Number of standard style classes (`style_NUMSTYLES`).
pub const NUMSTYLES: u32 = 11;

/// Colour resolved from a window type's `stylehint_TextColor` (7),
/// `stylehint_BackColor` (8), and `stylehint_ReverseColor` (9) for one style
/// class. `fg`/`bg` are 24-bit RGB (`0xRRGGBB`); `None` means no hint is set, so
/// the host uses its own default. `reverse` swaps foreground/background on
/// display. Plain data — keeps `gvm` zero-dependency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleColour {
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub reverse: bool,
}

/// The non-colour stylehints this Glk subset records for `glk_style_distinguish`:
/// `stylehint_Weight` (4) and `stylehint_Oblique` (5) — the bold / italic hints a
/// terminal can render, so two styles set to different weights or obliques are
/// visually distinguishable. `None` = no hint set. The remaining hints
/// (Size, Indentation, ParaIndentation, Justification, Proportional) describe
/// typographic properties a fixed-cell terminal cannot render, so they are not
/// recorded and never make styles distinguishable. Plain data — keeps `gvm`
/// zero-dependency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StyleAttrs {
    pub weight: Option<u32>,
    pub oblique: Option<u32>,
}

impl GlkStyle {
    /// Map a style number to a class (out-of-range falls back to Normal).
    pub fn from_num(v: u32) -> GlkStyle {
        match v {
            1 => GlkStyle::Emphasized,
            2 => GlkStyle::Preformatted,
            3 => GlkStyle::Header,
            4 => GlkStyle::Subheader,
            5 => GlkStyle::Alert,
            6 => GlkStyle::Note,
            7 => GlkStyle::BlockQuote,
            8 => GlkStyle::Input,
            9 => GlkStyle::User1,
            10 => GlkStyle::User2,
            _ => GlkStyle::Normal,
        }
    }
    /// The style number for this class.
    pub fn to_num(self) -> u32 {
        match self {
            GlkStyle::Normal => 0,
            GlkStyle::Emphasized => 1,
            GlkStyle::Preformatted => 2,
            GlkStyle::Header => 3,
            GlkStyle::Subheader => 4,
            GlkStyle::Alert => 5,
            GlkStyle::Note => 6,
            GlkStyle::BlockQuote => 7,
            GlkStyle::Input => 8,
            GlkStyle::User1 => 9,
            GlkStyle::User2 => 10,
        }
    }
}

// ── Event types + keycodes (`evtype_*`, `keycode_*`, from glk.h) ──────────────

/// Glk event type codes (`evtype_*`), as written into the `type` field of the
/// `event_t` struct delivered by `glk_select`. (Timer/Sound/Hyperlink are listed
/// for completeness; this subset delivers only Char/Line/Arrange.)
pub mod evtype {
    /// `evtype_None` — placeholder; never returned by a (blocking) `glk_select`.
    pub const NONE: u32 = 0;
    /// `evtype_Timer`.
    pub const TIMER: u32 = 1;
    /// `evtype_CharInput` — a single keystroke.
    pub const CHAR_INPUT: u32 = 2;
    /// `evtype_LineInput` — a completed line of text.
    pub const LINE_INPUT: u32 = 3;
    /// `evtype_MouseInput`.
    pub const MOUSE_INPUT: u32 = 4;
    /// `evtype_Arrange` — window sizes changed.
    pub const ARRANGE: u32 = 5;
    /// `evtype_Redraw`.
    pub const REDRAW: u32 = 6;
    /// `evtype_SoundNotify`.
    pub const SOUND_NOTIFY: u32 = 7;
    /// `evtype_Hyperlink`.
    pub const HYPERLINK: u32 = 8;
    /// `evtype_VolumeNotify` — a gradual `glk_schannel_set_volume_ext` change
    /// completed (Glk 0.7.3 Sound2). `win` is NULL, `val1` is 0, `val2` is the
    /// game's notify value.
    pub const VOLUME_NOTIFY: u32 = 9;
}

/// Glk special keycodes for character input (`keycode_*`). These occupy the top
/// of the 32-bit range: `keycode_Func12` (`0xffff_ffe4`) up to `keycode_Unknown`
/// (`0xffff_ffff`) — `keycode_MAXVAL` (28) distinct codes. A non-Unicode char
/// request reports a Latin-1 code (≤ 0xff) or one of these; anything else
/// becomes [`keycode::UNKNOWN`].
pub mod keycode {
    /// `keycode_Unknown`.
    pub const UNKNOWN: u32 = 0xffff_ffff;
    /// `keycode_Left`.
    pub const LEFT: u32 = 0xffff_fffe;
    /// `keycode_Right`.
    pub const RIGHT: u32 = 0xffff_fffd;
    /// `keycode_Up`.
    pub const UP: u32 = 0xffff_fffc;
    /// `keycode_Down`.
    pub const DOWN: u32 = 0xffff_fffb;
    /// `keycode_Return` (Enter).
    pub const RETURN: u32 = 0xffff_fffa;
    /// `keycode_Delete` (Backspace).
    pub const DELETE: u32 = 0xffff_fff9;
    /// `keycode_Escape`.
    pub const ESCAPE: u32 = 0xffff_fff8;
    /// `keycode_Tab`.
    pub const TAB: u32 = 0xffff_fff7;
    /// `keycode_PageUp`.
    pub const PAGE_UP: u32 = 0xffff_fff6;
    /// `keycode_PageDown`.
    pub const PAGE_DOWN: u32 = 0xffff_fff5;
    /// `keycode_Home`.
    pub const HOME: u32 = 0xffff_fff4;
    /// `keycode_End`.
    pub const END: u32 = 0xffff_fff3;
    /// `keycode_Func1`.
    pub const FUNC1: u32 = 0xffff_ffef;
    /// `keycode_Func12` — the lowest-valued special keycode.
    pub const FUNC12: u32 = 0xffff_ffe4;
    /// `keycode_MAXVAL` — the number of special keycodes.
    pub const MAXVAL: u32 = 28;
    /// The lowest value in the special-keycode block (`keycode_Func12`); any
    /// `glui32` ≥ this is a special key rather than a Unicode code point.
    pub const SPECIAL_FLOOR: u32 = FUNC12;

    /// Whether `key` may be set as a line-input terminator (Glk spec §11.2 /
    /// `gestalt_LineTerminatorKey`): only `keycode_Escape` and the function keys
    /// `keycode_Func1`..`keycode_Func12`. Return, arrows, Delete, and Tab are
    /// reserved by the input editor and can never terminate a line.
    pub fn is_terminator(key: u32) -> bool {
        key == ESCAPE || (FUNC12..=FUNC1).contains(&key)
    }
}

// ── Date/time (Glk 0.7.6 §"The System Clock" + "Time and Date Conversions") ────

/// Pure civil-calendar and clock arithmetic for the Glk date/time selectors
/// (`glk_current_time`, `glk_*_to_date_*`, `glk_date_to_*`). Zero-dependency:
/// no `chrono`, no platform `libc` calls — the day↔year/month/day conversions
/// use Howard Hinnant's well-known `days_from_civil` / `civil_from_days`
/// algorithms (proleptic Gregorian, unlimited range), and "now" is read by the
/// caller from [`std::time::SystemTime`]. The semantics match cheapglk's
/// `cgdate.c` (the reference Glk implementation).
///
/// **UTC only.** `std` has no timezone database and this crate makes no unsafe
/// platform calls, so the `_local` selectors are implemented identically to the
/// `_utc` ones (some interpreters do the same). This is a documented limitation,
/// not a bug: absent a tz database there is no honest local answer to give.
pub mod datetime {
    /// A Glk `glktimeval_t`: seconds since the Unix epoch as a signed 64-bit
    /// count split into `high_sec` (top 32 bits, signed) and `low_sec` (bottom
    /// 32 bits, unsigned), plus a `microsec` fraction. Field order matches
    /// `glk.h`: `{ glsi32 high_sec; glui32 low_sec; glsi32 microsec; }`.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct GlkTimeVal {
        pub high_sec: i32,
        pub low_sec: u32,
        pub microsec: i32,
    }

    /// A Glk `glkdate_t` (field order per `glk.h`): full four-digit `year`,
    /// `month` 1–12 (1 = January), `day` 1–31, `weekday` 0–6 (0 = Sunday),
    /// `hour` 0–23, `minute` 0–59, `second` 0–59 (maybe 60 at a leap second),
    /// `microsec` 0–999999.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct GlkDate {
        pub year: i32,
        pub month: i32,
        pub day: i32,
        pub weekday: i32,
        pub hour: i32,
        pub minute: i32,
        pub second: i32,
        pub microsec: i32,
    }

    /// Days from the Unix epoch (1970-01-01) to civil date `y-m-d` (proleptic
    /// Gregorian). Hinnant's algorithm; linear in `d`, so an out-of-range day
    /// (e.g. Jan 32) normalizes forward exactly as `timegm` would. Requires
    /// `m` in 1..=12 (the caller normalizes month into range first).
    pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = (if y >= 0 { y } else { y - 399 }) / 400;
        let yoe = y - era * 400; // [0, 399]
        let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }

    /// Civil date `(year, month, day)` for a day count from the Unix epoch
    /// (inverse of [`days_from_civil`]). Hinnant's algorithm.
    pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
        let z = z + 719468;
        let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
        let doe = z - era * 146097; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
        let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
        (if m <= 2 { y + 1 } else { y }, m, d)
    }

    /// Day-of-week for a day count from the Unix epoch, 0 = Sunday. Epoch day 0
    /// (1970-01-01) was a Thursday (4).
    pub fn weekday_from_days(z: i64) -> i64 {
        (z % 7 + 4).rem_euclid(7)
    }

    /// Split a signed epoch-second count + `microsec` into a [`GlkTimeVal`]
    /// (`cgdate.c` `gli_timestamp_to_time`).
    pub fn timestamp_to_timeval(secs: i64, microsec: i32) -> GlkTimeVal {
        GlkTimeVal {
            high_sec: (secs >> 32) as i32,
            low_sec: (secs & 0xFFFF_FFFF) as u32,
            microsec,
        }
    }

    /// Recombine a [`GlkTimeVal`]'s `high_sec`/`low_sec` into a signed epoch
    /// second count (`cgdate.c`: `timestamp = low_sec; timestamp += high_sec<<32`).
    pub fn timeval_to_timestamp(tv: GlkTimeVal) -> i64 {
        ((tv.high_sec as i64) << 32) + (tv.low_sec as i64)
    }

    /// Convert an epoch second count + `microsec` to a broken-down UTC date
    /// (`glk_time_to_date_utc`). Seconds floor-divide toward the past for
    /// pre-1970 timestamps, so the time-of-day fields stay in range.
    pub fn time_to_date(secs: i64, microsec: i32) -> GlkDate {
        let days = secs.div_euclid(86400);
        let rem = secs.rem_euclid(86400);
        let (y, m, d) = civil_from_days(days);
        GlkDate {
            year: y as i32,
            month: m as i32,
            day: d as i32,
            weekday: weekday_from_days(days) as i32,
            hour: (rem / 3600) as i32,
            minute: (rem % 3600 / 60) as i32,
            second: (rem % 60) as i32,
            microsec,
        }
    }

    /// Normalize a (possibly out-of-range) [`GlkDate`] to an epoch second count
    /// and a normalized `microsec` in 0..=999999 (`glk_date_to_time_utc`).
    /// Mirrors cgdate.c's `gli_date_to_tm` microsecond carry, then reduces the
    /// broken-down fields to a single second count via [`days_from_civil`]
    /// (which absorbs out-of-range day/hour/minute/second linearly, as `timegm`
    /// does); month is carried into the year first so the day-of-year formula
    /// stays valid.
    pub fn date_to_time(date: GlkDate) -> (i64, i32) {
        let mut sec = date.second as i64;
        let mut microsec = date.microsec as i64;
        if microsec >= 1_000_000 {
            sec += microsec / 1_000_000;
            microsec %= 1_000_000;
        } else if microsec < 0 {
            microsec = -1 - microsec;
            sec -= 1 + microsec / 1_000_000;
            microsec = 999_999 - (microsec % 1_000_000);
        }
        let total_months = date.year as i64 * 12 + (date.month as i64 - 1);
        let ny = total_months.div_euclid(12);
        let nm = total_months.rem_euclid(12) + 1; // [1, 12]
        let days = days_from_civil(ny, nm, date.day as i64);
        let secs = days * 86400 + date.hour as i64 * 3600 + date.minute as i64 * 60 + sec;
        (secs, microsec as i32)
    }

    /// Divide an epoch second count by `factor`, rounding toward negative
    /// infinity (`glk_*_simple_time`, cgdate.c `gli_simplify_time`). `factor == 0`
    /// is invalid and yields 0.
    pub fn simplify_time(secs: i64, factor: u32) -> i32 {
        if factor == 0 {
            return 0;
        }
        secs.div_euclid(factor as i64) as i32
    }
}

/// A delivered Glk event: the four `glui32` words written at the `event_t*`
/// passed to `glk_select` — `type`, the window id (`winid_t`), and `val1`/`val2`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlkEvent {
    /// The `evtype_*` code.
    pub etype: u32,
    /// The window that generated the event (0 = none).
    pub win: u32,
    /// First event-specific value (line: char count; char: the key code).
    pub val1: u32,
    /// Second event-specific value (line terminator key, else 0).
    pub val2: u32,
}

impl GlkEvent {
    /// The `evtype_None` event (no event available).
    pub fn none() -> GlkEvent {
        GlkEvent { etype: evtype::NONE, win: 0, val1: 0, val2: 0 }
    }
}

/// A window's resolved rectangle, in characters, within the display.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    /// Left edge column (0-based).
    pub left: u32,
    /// Top edge row (0-based).
    pub top: u32,
    /// Width in characters.
    pub width: u32,
    /// Height in characters (rows).
    pub height: u32,
}

// ── Backend trait ─────────────────────────────────────────────────────────────

/// A display backend the VM drives for all output-side effects. The Glk state
/// (window tree, streams, current style) lives in [`Model`]; the backend renders
/// it. Every method has a no-op default so a backend implements only what it
/// needs; `as_any_mut` supports downcasting in tests.
pub trait GlkBackend {
    /// Total display size available to the root window, in characters
    /// `(width, height)`. The model lays the window tree out within this.
    fn screen_size(&self) -> (u32, u32) {
        (80, 24)
    }
    /// A window was opened.
    fn window_open(&mut self, _id: u32, _wintype: WinType) {}
    /// A window was closed.
    fn window_close(&mut self, _id: u32) {}
    /// The resolved layout changed: each entry is `(window id, type, rect,
    /// border)` for every non-pair window, in window-id order. `border` is the
    /// leaf's border-presence hint (SQ-0286): `None` = no preference (parentless
    /// root), `Some(false)` = `winmethod_NoBorder`, `Some(true)` = `winmethod_Border`.
    /// A host that draws inter-window frames uses it to force presence on/off.
    fn window_layout(&mut self, _wins: &[(u32, WinType, Rect, Option<bool>)]) {}
    /// Append `s` (already style-tagged) to a text-buffer window.
    fn put_text(&mut self, _win: u32, _style: GlkStyle, _s: &str) {}
    /// Write `s` to a text-grid window's cells starting at `(x, y)`.
    fn grid_put(&mut self, _win: u32, _x: u32, _y: u32, _style: GlkStyle, _s: &str) {}
    /// Append `s` to a text-buffer window with the colour resolved from the
    /// active style hints. Defaults to the colourless [`GlkBackend::put_text`],
    /// so backends that don't render colour need no change (mirrors the
    /// Z-machine `Output::print_attr` seam).
    fn put_text_attr(&mut self, win: u32, style: GlkStyle, _colour: StyleColour, _link: u32, s: &str) {
        self.put_text(win, style, s);
    }
    /// Write `s` to a text-grid window with resolved colour and hyperlink value;
    /// defaults to the colourless [`GlkBackend::grid_put`].
    fn grid_put_attr(&mut self, win: u32, x: u32, y: u32, style: GlkStyle, _colour: StyleColour, _link: u32, s: &str) {
        self.grid_put(win, x, y, style, s);
    }
    /// Clear a text-grid window.
    fn grid_clear(&mut self, _win: u32) {}
    /// Clear a text-buffer window.
    fn window_clear(&mut self, _win: u32) {}
    /// Pixel size of one character cell `(width, height)`, used to convert a
    /// graphics window's pixel geometry into terminal cells. Defaults to `(1, 1)`.
    fn char_pixels(&self) -> (u32, u32) {
        (1, 1)
    }
    /// Pixel dimensions of image resource `resnum`, if it exists and decodes.
    fn image_info(&mut self, _resnum: u32) -> Option<(u32, u32)> {
        None
    }
    /// The bytes of Blorb `Data` resource `num` plus a text-vs-binary flag
    /// (`true` = a `TEXT` chunk, `false` = `BINA`/`FORM` binary), for
    /// `glk_stream_open_resource`. `None` when the host has no such resource
    /// (no Blorb, or no `Data` chunk with that number) — the open then fails.
    fn data_resource(&mut self, _num: u32) -> Option<(Vec<u8>, bool)> {
        None
    }
    /// The colours the host actually renders by default for style class `style`
    /// (a `style_*` number) in a window of type `wintype`, as `(fg, bg)` — each
    /// channel `Some(0x00RRGGBB)` or `None` when it is the terminal's own
    /// default (unknowable, so no guess). The outer `None` means the host
    /// reports no rendered colours at all (the default). Consulted by
    /// `glk_style_measure` only for a colour hint the game did not set itself,
    /// so games (e.g. Kerkerkruip) can detect a dark host background and adapt
    /// (SQ-0315).
    fn default_style_colours(&self, _wintype: WinType, _style: u32) -> Option<(Option<u32>, Option<u32>)> {
        None
    }
    /// Fill a rectangle of a graphics window with `color`.
    fn graphics_fill_rect(&mut self, _win: u32, _color: u32, _left: i32, _top: i32, _w: u32, _h: u32) {}
    /// Erase a rectangle of a graphics window to its background color.
    fn graphics_erase_rect(&mut self, _win: u32, _left: i32, _top: i32, _w: u32, _h: u32) {}
    /// Set a graphics window's background color.
    fn graphics_set_background(&mut self, _win: u32, _color: u32) {}
    /// Draw image `resnum` into a graphics window at `(x, y)`, optionally
    /// scaled to `(width, height)`. Return whether the image actually
    /// resolved and was drawn (false if `resnum` is missing/undecodable).
    fn graphics_draw_image(&mut self, _win: u32, _resnum: u32, _x: i32, _y: i32, _scale: Option<(u32, u32)>) -> bool {
        false
    }
    /// Create a sound channel with rock `rock`; return its Glk ref (0 = failure).
    fn schannel_create(&mut self, _rock: u32) -> u32 { 0 }
    /// Destroy a sound channel.
    fn schannel_destroy(&mut self, _chan: u32) {}
    /// Iterate channels: `chan == 0` → first; else the channel after `chan`.
    /// Return `(next_ref_or_0, that_channel_rock_or_0)`.
    fn schannel_iterate(&mut self, _chan: u32) -> (u32, u32) { (0, 0) }
    /// The rock of channel `chan` (0 if unknown).
    fn schannel_get_rock(&mut self, _chan: u32) -> u32 { 0 }
    /// Play sound resource `snd` on `chan`, `repeats` times (0xFFFFFFFF = forever),
    /// posting an `Evtype_SoundNotify` with value `notify` on completion when
    /// `notify != 0`. Return 1 on success, 0 on failure.
    fn schannel_play(&mut self, _chan: u32, _snd: u32, _repeats: u32, _notify: u32) -> u32 { 0 }
    /// Stop whatever is playing on `chan` (no notify is posted for a stop).
    fn schannel_stop(&mut self, _chan: u32) {}
    /// Set `chan`'s volume (Glk scale: 0x10000 = full).
    fn schannel_set_volume(&mut self, _chan: u32, _vol: u32) {}
    /// Create a sound channel with an initial `volume` (Glk scale, 0x10000 =
    /// full) already applied (Sound2 `glk_schannel_create_ext`). Default: create
    /// then set the volume, so a backend that only overrode the base calls still
    /// gets the initial level.
    fn schannel_create_ext(&mut self, rock: u32, volume: u32) -> u32 {
        let chan = self.schannel_create(rock);
        if chan != 0 {
            self.schannel_set_volume(chan, volume);
        }
        chan
    }
    /// Pause playback on `chan`, retaining position (Sound2 `glk_schannel_pause`);
    /// a sound later started on a paused channel begins paused. Default no-op.
    fn schannel_pause(&mut self, _chan: u32) {}
    /// Resume a paused `chan` from where it left off (Sound2
    /// `glk_schannel_unpause`). Default no-op.
    fn schannel_unpause(&mut self, _chan: u32) {}
    /// Change `chan`'s volume to `vol`, ramped over `duration_ms` (0 = immediate)
    /// (Sound2 `glk_schannel_set_volume_ext`). When `notify != 0` the HOST must
    /// deliver an `evtype_VolumeNotify(val2 = notify)` once the ramp completes
    /// (see [`crate::exec::Machine::deliver_volume_notify`]); the model does not
    /// hold a clock. Default: immediate set (the host owns the ramp + notify).
    fn schannel_set_volume_ext(&mut self, chan: u32, vol: u32, _duration_ms: u32, _notify: u32) {
        self.schannel_set_volume(chan, vol);
    }
    /// Flush any buffered output to the display.
    fn flush(&mut self) {}
    /// Immutable downcast support (used by tests to read recorded output).
    fn as_any(&self) -> &dyn Any;
    /// Mutable downcast support.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ── Test backend ──────────────────────────────────────────────────────────────

/// One recorded `fill_rect`/`erase_rect` call: `(color, left, top, w, h)`.
type FillRec = (u32, i32, i32, u32, u32);
/// One recorded `draw_image` call: `(resnum, x, y, scale)`.
type DrawRec = (u32, i32, i32, Option<(u32, u32)>);

/// A [`GlkBackend`] that records each window's text/grid in memory, replacing
/// the old `BufferOutput`: tests downcast to it and read the asserted strings.
pub struct TestBackend {
    /// Reported display size.
    pub screen: (u32, u32),
    /// Reported character-cell pixel size (w, h). Defaults to (1, 1).
    pub char_px: (u32, u32),
    /// Styled output runs per text-buffer window id, in print order.
    runs: BTreeMap<u32, Vec<(GlkStyle, String)>>,
    /// Styled output runs with their hyperlink value `(style, link, text)` per
    /// text-buffer window id, in print order (0 link = no hyperlink).
    linked_runs: BTreeMap<u32, Vec<(GlkStyle, u32, String)>>,
    /// Styled output runs with their resolved colour `(style, colour, text)` per
    /// text-buffer window id, in print order (records the garglk override result).
    colour_runs: BTreeMap<u32, Vec<(GlkStyle, StyleColour, String)>>,
    /// Grid cells per text-grid window id, keyed `(row, col) -> char`.
    grid: BTreeMap<u32, BTreeMap<(u32, u32), char>>,
    /// Last laid-out rect per window id.
    dims: BTreeMap<u32, Rect>,
    /// Recorded `fill_rect`/`erase_rect` calls per graphics window.
    fills: BTreeMap<u32, Vec<FillRec>>,
    /// Recorded `draw_image` calls per graphics window.
    draws: BTreeMap<u32, Vec<DrawRec>>,
    /// Resnums that simulate a missing/undecodable image (draw reports false,
    /// nothing recorded).
    missing_images: BTreeSet<u32>,
    /// Last background color set per graphics window.
    backgrounds: BTreeMap<u32, u32>,
    /// Next schannel ref to hand out (pre-incremented; first create → 1).
    next_schannel: u32,
    /// Rock per live schannel ref.
    schannel_rocks: BTreeMap<u32, u32>,
    /// Human-readable log of schannel calls, in order (for dispatch assertions).
    sound_log: Vec<String>,
    /// Canned Blorb `Data` resources by number → `(bytes, is_text)`, served by
    /// [`GlkBackend::data_resource`] for `glk_stream_open_resource` tests.
    data_resources: BTreeMap<u32, (Vec<u8>, bool)>,
    /// Canned host-rendered default style colours `(fg, bg)`, served by
    /// [`GlkBackend::default_style_colours`] for every wintype/style when set.
    default_colours: Option<(Option<u32>, Option<u32>)>,
}

impl Default for TestBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TestBackend {
    /// A backend with a default 80×24 display.
    pub fn new() -> Self {
        TestBackend {
            screen: (80, 24),
            char_px: (1, 1),
            runs: BTreeMap::new(),
            linked_runs: BTreeMap::new(),
            colour_runs: BTreeMap::new(),
            grid: BTreeMap::new(),
            dims: BTreeMap::new(),
            fills: BTreeMap::new(),
            draws: BTreeMap::new(),
            missing_images: BTreeSet::new(),
            backgrounds: BTreeMap::new(),
            next_schannel: 0,
            schannel_rocks: BTreeMap::new(),
            sound_log: Vec::new(),
            data_resources: BTreeMap::new(),
            default_colours: None,
        }
    }
    /// A backend reporting a specific display size.
    pub fn with_screen(width: u32, height: u32) -> Self {
        TestBackend { screen: (width, height), ..Self::new() }
    }
    /// A backend reporting a specific character-cell pixel size.
    pub fn with_char_pixels(mut self, cw: u32, ch: u32) -> Self {
        self.char_px = (cw, ch);
        self
    }
    /// Mark `resnum` as missing/undecodable: `graphics_draw_image` reports
    /// false for it and records nothing.
    pub fn with_missing_image(mut self, resnum: u32) -> Self {
        self.missing_images.insert(resnum);
        self
    }
    /// Register a canned Blorb `Data` resource (`num` → `(bytes, is_text)`) that
    /// [`GlkBackend::data_resource`] will serve to `glk_stream_open_resource`.
    pub fn with_data_resource(mut self, num: u32, bytes: Vec<u8>, is_text: bool) -> Self {
        self.data_resources.insert(num, (bytes, is_text));
        self
    }
    /// Set the canned host-rendered default style colours `(fg, bg)` that
    /// [`GlkBackend::default_style_colours`] reports for every wintype/style.
    pub fn with_default_colours(mut self, fg: Option<u32>, bg: Option<u32>) -> Self {
        self.default_colours = Some((fg, bg));
        self
    }
    /// Accumulated text for one text-buffer window (empty if none).
    pub fn text(&self, win: u32) -> String {
        self.runs
            .get(&win)
            .map(|rs| rs.iter().map(|(_, s)| s.as_str()).collect())
            .unwrap_or_default()
    }
    /// The styled output runs recorded for one text-buffer window.
    pub fn runs(&self, win: u32) -> Vec<(GlkStyle, String)> {
        self.runs.get(&win).cloned().unwrap_or_default()
    }
    /// The styled output runs with their hyperlink value `(style, link, text)`
    /// recorded for one text-buffer window.
    pub fn linked_runs(&self, win: u32) -> Vec<(GlkStyle, u32, String)> {
        self.linked_runs.get(&win).cloned().unwrap_or_default()
    }
    /// The styled output runs with their resolved colour `(style, colour, text)`
    /// recorded for one text-buffer window (reflects any garglk override).
    pub fn colour_runs(&self, win: u32) -> Vec<(GlkStyle, StyleColour, String)> {
        self.colour_runs.get(&win).cloned().unwrap_or_default()
    }
    /// All text-buffer windows' text concatenated in window-id order — the
    /// migration replacement for `BufferOutput::buf` (there is one window in the
    /// migrated tests, so this is exactly that window's text).
    pub fn all_text(&self) -> String {
        self.runs
            .values()
            .flat_map(|rs| rs.iter().map(|(_, s)| s.as_str()))
            .collect()
    }
    /// The resolved rect last reported for a window.
    pub fn rect(&self, win: u32) -> Option<Rect> {
        self.dims.get(&win).copied()
    }
    /// One row of a text-grid window as a string (trailing spaces trimmed).
    pub fn grid_line(&self, win: u32, row: u32) -> String {
        let Some(cells) = self.grid.get(&win) else { return String::new() };
        let width = self.dims.get(&win).map(|r| r.width).unwrap_or(0);
        let mut s = String::new();
        for col in 0..width {
            s.push(cells.get(&(row, col)).copied().unwrap_or(' '));
        }
        s.trim_end().to_string()
    }
    /// Recorded `fill_rect`/`erase_rect` calls for one graphics window (empty if none).
    pub fn fills(&self, win: u32) -> Vec<FillRec> {
        self.fills.get(&win).cloned().unwrap_or_default()
    }
    /// Recorded `draw_image` calls for one graphics window (empty if none).
    pub fn draws(&self, win: u32) -> Vec<DrawRec> {
        self.draws.get(&win).cloned().unwrap_or_default()
    }
    /// The last background color set for one graphics window, if any.
    pub fn background(&self, win: u32) -> Option<u32> {
        self.backgrounds.get(&win).copied()
    }
    /// The recorded schannel call log (create/play/stop/setvol/destroy), in order.
    pub fn sound_log(&self) -> &[String] {
        &self.sound_log
    }
}

impl GlkBackend for TestBackend {
    fn screen_size(&self) -> (u32, u32) {
        self.screen
    }
    fn char_pixels(&self) -> (u32, u32) {
        self.char_px
    }
    fn window_open(&mut self, id: u32, wintype: WinType) {
        match wintype {
            WinType::TextBuffer => {
                self.runs.entry(id).or_default();
            }
            WinType::TextGrid => {
                self.grid.entry(id).or_default();
            }
            WinType::Pair => {}
            WinType::Graphics => {}
        }
    }
    fn window_close(&mut self, id: u32) {
        self.runs.remove(&id);
        self.linked_runs.remove(&id);
        self.colour_runs.remove(&id);
        self.grid.remove(&id);
        self.dims.remove(&id);
    }
    fn window_layout(&mut self, wins: &[(u32, WinType, Rect, Option<bool>)]) {
        for &(id, _ty, rect, _border) in wins {
            self.dims.insert(id, rect);
        }
    }
    fn put_text(&mut self, win: u32, style: GlkStyle, s: &str) {
        self.runs.entry(win).or_default().push((style, s.to_string()));
    }
    fn put_text_attr(&mut self, win: u32, style: GlkStyle, colour: StyleColour, link: u32, s: &str) {
        self.runs.entry(win).or_default().push((style, s.to_string()));
        self.linked_runs.entry(win).or_default().push((style, link, s.to_string()));
        self.colour_runs.entry(win).or_default().push((style, colour, s.to_string()));
    }
    fn grid_put(&mut self, win: u32, x: u32, y: u32, _style: GlkStyle, s: &str) {
        let cells = self.grid.entry(win).or_default();
        for (i, ch) in s.chars().enumerate() {
            cells.insert((y, x + i as u32), ch);
        }
    }
    fn grid_clear(&mut self, win: u32) {
        if let Some(cells) = self.grid.get_mut(&win) {
            cells.clear();
        }
    }
    fn window_clear(&mut self, win: u32) {
        if let Some(rs) = self.runs.get_mut(&win) {
            rs.clear();
        }
        if let Some(rs) = self.linked_runs.get_mut(&win) {
            rs.clear();
        }
        if let Some(rs) = self.colour_runs.get_mut(&win) {
            rs.clear();
        }
    }
    fn graphics_fill_rect(&mut self, win: u32, color: u32, left: i32, top: i32, w: u32, h: u32) {
        self.fills.entry(win).or_default().push((color, left, top, w, h));
    }
    fn graphics_erase_rect(&mut self, win: u32, left: i32, top: i32, w: u32, h: u32) {
        // erase records as a fill with the window's background (or 0).
        let color = self.backgrounds.get(&win).copied().unwrap_or(0);
        self.fills.entry(win).or_default().push((color, left, top, w, h));
    }
    fn graphics_set_background(&mut self, win: u32, color: u32) {
        self.backgrounds.insert(win, color);
    }
    fn graphics_draw_image(&mut self, win: u32, resnum: u32, x: i32, y: i32, scale: Option<(u32, u32)>) -> bool {
        if self.missing_images.contains(&resnum) {
            return false;
        }
        self.draws.entry(win).or_default().push((resnum, x, y, scale));
        true
    }
    fn schannel_create(&mut self, rock: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannel_rocks.insert(id, rock);
        self.sound_log.push(format!("create rock={rock} -> {id}"));
        id
    }
    fn schannel_destroy(&mut self, chan: u32) {
        self.schannel_rocks.remove(&chan);
        self.sound_log.push(format!("destroy chan={chan}"));
    }
    fn schannel_iterate(&mut self, chan: u32) -> (u32, u32) {
        let next = if chan == 0 {
            self.schannel_rocks.keys().next().copied()
        } else {
            self.schannel_rocks.range((chan + 1)..).next().map(|(k, _)| *k)
        };
        match next {
            Some(id) => (id, *self.schannel_rocks.get(&id).unwrap_or(&0)),
            None => (0, 0),
        }
    }
    fn schannel_get_rock(&mut self, chan: u32) -> u32 {
        *self.schannel_rocks.get(&chan).unwrap_or(&0)
    }
    fn schannel_play(&mut self, chan: u32, snd: u32, repeats: u32, notify: u32) -> u32 {
        self.sound_log.push(format!("play chan={chan} snd={snd} repeats={repeats} notify={notify}"));
        1
    }
    fn schannel_stop(&mut self, chan: u32) {
        self.sound_log.push(format!("stop chan={chan}"));
    }
    fn schannel_set_volume(&mut self, chan: u32, vol: u32) {
        self.sound_log.push(format!("setvol chan={chan} vol={vol}"));
    }
    fn schannel_create_ext(&mut self, rock: u32, volume: u32) -> u32 {
        self.next_schannel += 1;
        let id = self.next_schannel;
        self.schannel_rocks.insert(id, rock);
        self.sound_log.push(format!("create_ext rock={rock} vol={volume} -> {id}"));
        id
    }
    fn schannel_pause(&mut self, chan: u32) {
        self.sound_log.push(format!("pause chan={chan}"));
    }
    fn schannel_unpause(&mut self, chan: u32) {
        self.sound_log.push(format!("unpause chan={chan}"));
    }
    fn schannel_set_volume_ext(&mut self, chan: u32, vol: u32, duration_ms: u32, notify: u32) {
        self.sound_log
            .push(format!("setvol_ext chan={chan} vol={vol} dur={duration_ms} notify={notify}"));
    }
    fn data_resource(&mut self, num: u32) -> Option<(Vec<u8>, bool)> {
        self.data_resources.get(&num).cloned()
    }
    fn default_style_colours(&self, _wintype: WinType, _style: u32) -> Option<(Option<u32>, Option<u32>)> {
        self.default_colours
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ── The model ─────────────────────────────────────────────────────────────────

/// What a stream writes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StreamKind {
    /// A window's output stream (text routed to that window).
    Window(u32),
    /// A Glulx-memory stream: bytes/words land in main memory `[addr, addr+len)`.
    /// `unicode` selects 32-bit elements; `pos` is the element cursor.
    Memory { addr: u32, len: u32, pos: u32, unicode: bool },
    /// A file stream over the in-memory VFS. `unicode` selects the on-file
    /// encoding (4-byte-BE / UTF-8 vs 1 byte per char); the mutable name/mode/pos
    /// state lives in [`Model::file_streams`] keyed by stream id so this stays `Copy`.
    File { unicode: bool },
    /// A `SavedGame`-usage stream: a host conduit fully decoupled from the VFS.
    /// Opens successfully for every mode (Read succeeds even with no prior save,
    /// so the game always reaches `@save`/`@restore` and the host decides).
    /// Writes discard, reads return EOF; no `self.files`/`file_streams` entry.
    Null,
    /// A read-only Blorb data-resource stream (`glk_stream_open_resource[_uni]`).
    /// The resource bytes + read cursor + text/binary flag live in
    /// [`Model::resource_streams`] keyed by stream id so this stays `Copy`;
    /// `unicode` selects 32-bit vs Latin-1 read elements.
    Resource { unicode: bool },
}

/// A Glk stream.
#[derive(Clone, Debug)]
struct Stream {
    id: u32,
    rock: u32,
    kind: StreamKind,
    style: GlkStyle,
    /// The current Glk hyperlink value for text written to this stream
    /// (`glk_set_hyperlink`); 0 = no link. Stamped onto each output run.
    link: u32,
    /// Gargoyle `garglk_set_zcolors` / `garglk_set_reversevideo` overrides for
    /// text written to this stream. `None` = no override (use the style hint's
    /// colour); `Some` forces this fg/bg/reverse. Not persisted in the snapshot
    /// (mirrors `style_hints`, which also reset to defaults on restore).
    zfg: Option<u32>,
    zbg: Option<u32>,
    zrev: Option<bool>,
    read_count: u32,
    write_count: u32,
}

/// Text-grid cursor + dimensions (the cells themselves live in the backend).
#[derive(Clone, Copy, Default)]
struct Grid {
    width: u32,
    height: u32,
    cx: u32,
    cy: u32,
}

/// A pending line-input request on a window (`glk_request_line_event`). The
/// engine fills `buf` (in Glulx memory) when the host supplies the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineReq {
    /// Glulx address of the input buffer.
    pub buf: u32,
    /// Maximum element count the buffer holds.
    pub maxlen: u32,
    /// Pre-filled element count already in the buffer at request time.
    pub initlen: u32,
    /// `true` for `_uni` (32-bit elements); `false` for Latin-1 bytes.
    pub unicode: bool,
}

/// A pending character-input request on a window (`glk_request_char_event`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharReq {
    /// `true` for `_uni` (full Unicode code point); `false` reports Latin-1 or a
    /// special keycode (else `keycode_Unknown`).
    pub unicode: bool,
}

/// A Glk window (tree node).
#[derive(Clone)]
struct Window {
    id: u32,
    wintype: WinType,
    rock: u32,
    parent: u32,
    /// The window's own output stream (0 for pair windows).
    stream: u32,
    rect: Rect,
    grid: Grid,
    /// A pending line-input request (None when not awaiting a line).
    line_req: Option<LineReq>,
    /// A pending char-input request (None when not awaiting a key).
    char_req: Option<CharReq>,
    /// A pending mouse-input request (`glk_request_mouse_event`). Unlike a char
    /// request this is a bare flag: Glk mouse events carry no per-request option.
    mouse_req: bool,
    /// A pending hyperlink-input request (`glk_request_hyperlink_event`). Like a
    /// mouse request this is a bare flag: a Glk hyperlink event carries no
    /// per-request option.
    hyperlink_req: bool,
    /// The line-input terminator keycodes set for this window
    /// (`glk_set_terminators_line_event`); persists across line requests until
    /// reset. Empty = Enter-only (the default).
    terminators: Vec<u32>,
    /// Whether completed line input is echoed (`glk_set_echo_line_event`).
    /// Default true (Glk spec §4.2). Hosts that echo typed input consult this;
    /// a game that turns it off takes responsibility for echoing itself.
    echo_line: bool,
    /// The window's echo stream (`glk_window_set_echo_stream`; 0 = none). Every
    /// character printed to this window is also written to this stream (Glk spec
    /// §3.6). Cleared to 0 when that stream is closed; not cleared when the
    /// window itself closes (the echo stream outlives the window). Not
    /// serialized (rebuilt by the game after a restore, like other stream links).
    echo: u32,
    // Pair-window fields (all 0 for leaf windows):
    child1: u32,
    child2: u32,
    key: u32,
    method: u32,
    size: u32,
}

/// A Glk file reference: a named handle into the in-memory VFS (`Model::files`).
/// Filerefs are indexed by `id - 1` in `Model::filerefs` (None = a freed slot).
#[derive(Clone, Debug)]
struct FileRef {
    id: u32,
    rock: u32,
    name: String,
    usage: u32,
    /// `true` only for a `glk_fileref_create_by_prompt` fileref (the player's
    /// own SAVE/RESTORE verb): its `@save`/`@restore` should surface the host's
    /// save UI. `create_by_name`/`create_temp`/`create_from` filerefs (the
    /// game's own fixed-name saves — CM's init cache, undo, autotesting) are
    /// `false`: the host services them silently against a fixed path. Not
    /// serialized (defaults to `false` on snapshot restore).
    by_prompt: bool,
}

/// Glk `fileusage_TextMode` bit (garglk `glk.h`): when set on a fileref's usage,
/// a file stream over it uses the text encoding (UTF-8 for unicode streams);
/// clear = binary (4-byte big-endian for unicode streams).
#[allow(non_upper_case_globals)]
const fileusage_TextMode: u32 = 0x100;

/// The mutable read/write state of an open file stream, kept in a side table
/// (`Model::file_streams`) keyed by stream id so `StreamKind` stays `Copy`.
#[derive(Clone, Debug)]
struct FileStream {
    name: String,
    /// The open filemode; retained for the snapshot round-trip (Task 3). Encoding
    /// is derived from `unicode` + `usage`, so read/write don't consult it.
    #[allow(dead_code)]
    mode: u32,
    pos: usize,
    unicode: bool,
    usage: u32,
    /// Whether the originating fileref was `create_by_prompt` (the player's SAVE
    /// verb). Lets `@save`/`@restore` over a VFS file stream (a game that saves to
    /// a Data-usage fileref, e.g. CM's init cache) route silently vs. via the
    /// host UI. Not serialized (defaults `false` on snapshot restore).
    by_prompt: bool,
}

/// The Glk window/stream model.
pub struct Model {
    /// Windows indexed by `id - 1` (None = a freed slot).
    windows: Vec<Option<Window>>,
    /// Streams indexed by `id - 1`.
    streams: Vec<Option<Stream>>,
    /// The in-memory virtual filesystem: file name → contents. Filerefs and file
    /// streams read and write these blobs; no real disk I/O occurs.
    files: std::collections::BTreeMap<String, Vec<u8>>,
    /// File references indexed by `id - 1` (None = a freed slot).
    filerefs: Vec<Option<FileRef>>,
    /// Open file streams keyed by stream id (the mutable read/write cursor state
    /// for each [`StreamKind::File`] stream).
    file_streams: std::collections::BTreeMap<u32, FileStream>,
    /// Root window id (0 = no windows open).
    root: u32,
    /// Current output stream id (0 = none).
    cur_stream: u32,
    /// Queued non-input events (arrange/redraw) awaiting the next `glk_select`.
    events: std::collections::VecDeque<GlkEvent>,
    /// Colour style hints, indexed `[row][style]` where row 0 = text-buffer and
    /// row 1 = text-grid (see [`Model::hint_row`]). Set via `glk_stylehint_set`.
    style_hints: [[StyleColour; NUMSTYLES as usize]; 2],
    /// Non-colour rendered stylehints (Weight, Oblique), same `[row][style]`
    /// indexing as `style_hints`. Compared by `glk_style_distinguish`.
    style_attrs: [[StyleAttrs; NUMSTYLES as usize]; 2],
    /// Pixel size of one character cell (w, h), set by `relayout` for graphics
    /// fixed-split conversion. (1,1) until the backend reports otherwise.
    char_px: (u32, u32),
    /// Set whenever the file VFS is mutated (a file is created/truncated,
    /// written, or deleted), so hosts can flush the sidecar only when it
    /// changed. Mirrors the Z-machine's `aux_dirty`. Not serialized.
    vfs_dirty: bool,
    /// `SavedGame`-usage files that have been written this session, mapped to the
    /// byte length of their most recent `@save`. Those saves live in host `.qzl`
    /// files (decoupled from the VFS — see [`StreamKind::Null`]), so they never
    /// appear in `files`. This table lets the emulated Glk answer the game's
    /// post-`@save` verification (CounterfeitMonkey's SAVE verb): (1)
    /// `glk_fileref_does_file_exist` returns true for a written slot, and (2)
    /// reopening the slot for reading and seeking to its end reports the save's
    /// byte count — without it both read as "empty/absent" and the game prints
    /// "Save failed." even though the host wrote the file. Session-transient (not
    /// serialized), like `vfs_dirty`.
    saved_game_files: std::collections::BTreeMap<String, u32>,
    /// Per-`SavedGame` `Null`-stream state, keyed by stream id. Mirrors
    /// [`Model::file_streams`] (keeps [`StreamKind`] `Copy`). Lets
    /// `glk_stream_get_position`/`glk_stream_set_position` behave over the
    /// host-managed save's known length (from `saved_game_files`), and carries
    /// the originating fileref's `by_prompt` so `@save`/`@restore` can tell the
    /// player's SAVE verb from the game's own fixed-name saves.
    savegame_streams: std::collections::BTreeMap<u32, SaveStream>,
    /// Open Blorb data-resource streams keyed by stream id (the owned bytes +
    /// read cursor for each [`StreamKind::Resource`] stream).
    resource_streams: std::collections::BTreeMap<u32, ResourceStream>,
}

/// The read state of an open Blorb data-resource stream (see
/// [`StreamKind::Resource`]), kept in a side table (`Model::resource_streams`)
/// keyed by stream id so `StreamKind` stays `Copy`. Read-only.
#[derive(Clone, Debug)]
struct ResourceStream {
    /// The resource chunk's bytes (owned copy handed over by the host at open).
    data: Vec<u8>,
    /// Current byte cursor.
    pos: usize,
    /// `true` for a `_uni` stream (32-bit read elements); `false` for Latin-1.
    unicode: bool,
    /// `true` for a `TEXT` chunk — always UTF-8-decoded, with non-unicode reads
    /// clamping code points above 0xFF to `'?'` (cheapglk `cgstream.c`); `false`
    /// for a `BINA`/`FORM` binary chunk (raw byte / 4-byte big-endian when
    /// unicode).
    text: bool,
}

/// Per-`Null`-stream state for a host-managed `SavedGame` slot (see
/// [`Model::savegame_streams`]).
#[derive(Clone, Debug)]
struct SaveStream {
    /// The (sanitized) fileref name this stream was opened on.
    name: String,
    /// Current byte cursor (over the slot's known length).
    pos: u32,
    /// Whether the fileref was `create_by_prompt` (the player's SAVE verb).
    by_prompt: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// A fresh, empty model (no windows, no streams).
    pub fn new() -> Model {
        Model {
            windows: Vec::new(),
            streams: Vec::new(),
            files: std::collections::BTreeMap::new(),
            filerefs: Vec::new(),
            file_streams: std::collections::BTreeMap::new(),
            root: 0,
            cur_stream: 0,
            events: std::collections::VecDeque::new(),
            style_hints: [[StyleColour::default(); NUMSTYLES as usize]; 2],
            style_attrs: [[StyleAttrs::default(); NUMSTYLES as usize]; 2],
            char_px: (1, 1),
            vfs_dirty: false,
            saved_game_files: std::collections::BTreeMap::new(),
            savegame_streams: std::collections::BTreeMap::new(),
            resource_streams: std::collections::BTreeMap::new(),
        }
    }

    /// The `style_hints` rows a `wintype` argument touches: buffer (3) → [0],
    /// grid (4) → [1], `wintype_AllTypes` (0) → both. Unsupported types → none.
    fn hint_rows(wintype: u32) -> &'static [usize] {
        match wintype {
            0 => &[0, 1],
            3 => &[0],
            4 => &[1],
            _ => &[],
        }
    }

    /// Record a `glk_stylehint_set(wintype, styl, hint, val)`. The rendered hints
    /// are kept: TextColor (7), BackColor (8), ReverseColor (9) as colour, and
    /// Weight (4) / Oblique (5) as attributes; other hints and out-of-range
    /// styles are ignored.
    pub fn set_style_hint(&mut self, wintype: u32, styl: u32, hint: u32, val: u32) {
        if styl >= NUMSTYLES {
            return;
        }
        for &row in Self::hint_rows(wintype) {
            let sc = &mut self.style_hints[row][styl as usize];
            match hint {
                7 => sc.fg = Some(val & 0x00FF_FFFF),
                8 => sc.bg = Some(val & 0x00FF_FFFF),
                9 => sc.reverse = val != 0,
                _ => {}
            }
            let sa = &mut self.style_attrs[row][styl as usize];
            match hint {
                4 => sa.weight = Some(val),
                5 => sa.oblique = Some(val),
                _ => {}
            }
        }
    }

    /// Undo a `glk_stylehint_clear(wintype, styl, hint)` for a recorded hint.
    pub fn clear_style_hint(&mut self, wintype: u32, styl: u32, hint: u32) {
        if styl >= NUMSTYLES {
            return;
        }
        for &row in Self::hint_rows(wintype) {
            let sc = &mut self.style_hints[row][styl as usize];
            match hint {
                7 => sc.fg = None,
                8 => sc.bg = None,
                9 => sc.reverse = false,
                _ => {}
            }
            let sa = &mut self.style_attrs[row][styl as usize];
            match hint {
                4 => sa.weight = None,
                5 => sa.oblique = None,
                _ => {}
            }
        }
    }

    /// Resolve the colour hints active for `style` in a window of type `wintype`.
    pub fn style_colour(&self, wintype: WinType, style: GlkStyle) -> StyleColour {
        let row = match wintype {
            WinType::TextBuffer => 0,
            WinType::TextGrid => 1,
            WinType::Pair | WinType::Graphics => return StyleColour::default(),
        };
        self.style_hints[row][style as usize]
    }

    /// Resolve the non-colour rendered attributes (Weight, Oblique) active for
    /// `style` in a window of type `wintype`.
    pub fn style_attrs(&self, wintype: WinType, style: GlkStyle) -> StyleAttrs {
        let row = match wintype {
            WinType::TextBuffer => 0,
            WinType::TextGrid => 1,
            WinType::Pair | WinType::Graphics => return StyleAttrs::default(),
        };
        self.style_attrs[row][style as usize]
    }

    // ── Gargoyle garglk_* colour overrides ──────────────────────────────────────
    //
    // `garglk_set_zcolors` fg/bg sentinels (glk.h `zcolor_*`): Transparent /
    // Cursor / Current leave the channel unchanged, Default clears the override,
    // and any other value is a low-24-bit RGB colour.
    const ZCOLOR_TRANSPARENT: u32 = 0xffff_fffc;
    const ZCOLOR_CURSOR: u32 = 0xffff_fffd;
    const ZCOLOR_CURRENT: u32 = 0xffff_fffe;
    const ZCOLOR_DEFAULT: u32 = 0xffff_ffff;

    /// Apply one `garglk_set_zcolors` channel value to an override slot.
    fn apply_zcolor(slot: &mut Option<u32>, val: u32) {
        match val {
            Self::ZCOLOR_TRANSPARENT | Self::ZCOLOR_CURSOR | Self::ZCOLOR_CURRENT => {}
            Self::ZCOLOR_DEFAULT => *slot = None,
            rgb => *slot = Some(rgb & 0x00FF_FFFF),
        }
    }

    /// `garglk_set_zcolors[_stream]`: set the fg/bg colour override that applies
    /// to subsequent text written to `strid` (invalid id is ignored).
    pub fn set_stream_zcolors(&mut self, strid: u32, fg: u32, bg: u32) {
        if let Some(s) = self.stream_mut(strid) {
            Self::apply_zcolor(&mut s.zfg, fg);
            Self::apply_zcolor(&mut s.zbg, bg);
        }
    }

    /// `garglk_set_reversevideo[_stream]`: force reverse-video on/off for
    /// subsequent text written to `strid` (invalid id is ignored).
    pub fn set_stream_reversevideo(&mut self, strid: u32, reverse: u32) {
        if let Some(s) = self.stream_mut(strid) {
            s.zrev = Some(reverse != 0);
        }
    }

    /// Resolve the effective colour for text written to `strid` in a `wintype`
    /// window with the given `style`, layering any `garglk_*` override on top of
    /// the style-hint colour (an unset override channel falls through to the hint).
    pub fn stream_style_colour(&self, strid: u32, wintype: WinType, style: GlkStyle) -> StyleColour {
        let mut sc = self.style_colour(wintype, style);
        if let Some(s) = self.stream(strid) {
            if s.zfg.is_some() {
                sc.fg = s.zfg;
            }
            if s.zbg.is_some() {
                sc.bg = s.zbg;
            }
            if let Some(r) = s.zrev {
                sc.reverse = r;
            }
        }
        sc
    }

    // ── style measurement (Glk 0.7.6 §5.5.2) ──────────────────────────────────

    /// `glk_style_measure`: the value of style `hint` for style class `styl` in
    /// window `win`, if the model actually knows it (`None` = "unknown" — the
    /// honest answer for hints this Glk subset does not track). Only the colour
    /// hints are recorded, so only they can be measured: `stylehint_TextColor`
    /// (7) and `stylehint_BackColor` (8) report a colour when one was set for
    /// that style, and `stylehint_ReverseColor` (9) reports the effective flag.
    /// Every other hint (size, weight, oblique, justification, indentation,
    /// proportional) is untracked and honestly reported as unknown.
    pub fn style_measure(&self, win: u32, styl: u32, hint: u32) -> Option<u32> {
        let wintype = self.window_type(win)?;
        if styl >= NUMSTYLES {
            return None;
        }
        let sc = self.style_colour(wintype, GlkStyle::from_num(styl));
        match hint {
            7 => sc.fg,                   // stylehint_TextColor
            8 => sc.bg,                   // stylehint_BackColor
            9 => Some(sc.reverse as u32), // stylehint_ReverseColor
            _ => None,                    // untracked hint -> honestly unknown
        }
    }

    /// `glk_style_distinguish`: whether style classes `s1` and `s2` differ in a
    /// hint the model records and the terminal renders in window `win` —
    /// foreground/background colour or reverse-video (`style_colour`) or the
    /// Weight / Oblique attributes (`style_attrs`). Hints a fixed-cell terminal
    /// cannot render (Size, Indentation, Justification, Proportional) are not
    /// recorded, so they never distinguish. Styles the model cannot tell apart
    /// report `false` — the honest answer given the recorded hints (an intrinsic
    /// class difference like a bold Header comes from the host's style config,
    /// which this VM layer cannot see). Same style number, or an invalid window,
    /// is likewise not distinguishable.
    pub fn style_distinguish(&self, win: u32, s1: u32, s2: u32) -> bool {
        let Some(wintype) = self.window_type(win) else {
            return false;
        };
        if s1 == s2 {
            return false;
        }
        let (a, b) = (GlkStyle::from_num(s1), GlkStyle::from_num(s2));
        self.style_colour(wintype, a) != self.style_colour(wintype, b)
            || self.style_attrs(wintype, a) != self.style_attrs(wintype, b)
    }

    // ── slot accessors ────────────────────────────────────────────────────────

    fn win(&self, id: u32) -> Option<&Window> {
        if id == 0 {
            return None;
        }
        self.windows.get((id - 1) as usize).and_then(|w| w.as_ref())
    }
    fn win_mut(&mut self, id: u32) -> Option<&mut Window> {
        if id == 0 {
            return None;
        }
        self.windows.get_mut((id - 1) as usize).and_then(|w| w.as_mut())
    }
    fn stream(&self, id: u32) -> Option<&Stream> {
        if id == 0 {
            return None;
        }
        self.streams.get((id - 1) as usize).and_then(|s| s.as_ref())
    }
    fn stream_mut(&mut self, id: u32) -> Option<&mut Stream> {
        if id == 0 {
            return None;
        }
        self.streams.get_mut((id - 1) as usize).and_then(|s| s.as_mut())
    }

    fn alloc_window(&mut self, wintype: WinType, rock: u32) -> u32 {
        let id = (self.windows.len() + 1) as u32;
        self.windows.push(Some(Window {
            id,
            wintype,
            rock,
            parent: 0,
            stream: 0,
            rect: Rect::default(),
            grid: Grid::default(),
            line_req: None,
            char_req: None,
            mouse_req: false,
            hyperlink_req: false,
            terminators: Vec::new(),
            echo_line: true,
            echo: 0,
            child1: 0,
            child2: 0,
            key: 0,
            method: 0,
            size: 0,
        }));
        id
    }
    fn alloc_stream(&mut self, kind: StreamKind, rock: u32) -> u32 {
        let id = (self.streams.len() + 1) as u32;
        self.streams.push(Some(Stream {
            id,
            rock,
            kind,
            style: GlkStyle::Normal,
            link: 0,
            zfg: None,
            zbg: None,
            zrev: None,
            read_count: 0,
            write_count: 0,
        }));
        id
    }

    fn fileref(&self, id: u32) -> Option<&FileRef> {
        if id == 0 {
            return None;
        }
        self.filerefs.get((id - 1) as usize).and_then(|f| f.as_ref())
    }

    // ── filerefs (in-memory VFS) ────────────────────────────────────────────────

    /// Keep the characters Glk libraries safely allow in a base filename (ASCII
    /// alphanumerics plus `-`, `_`, `.`); everything else becomes `_`. An empty
    /// result falls back to `"file"` so a name is never blank.
    pub fn sanitize_fileref_name(raw: &str) -> String {
        let cleaned: String = raw
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
            .collect();
        if cleaned.is_empty() {
            "file".to_string()
        } else {
            cleaned
        }
    }

    /// Allocate a fileref slot for a (already-chosen) name; returns its id.
    /// `by_prompt` marks a `create_by_prompt` fileref (the player's SAVE/RESTORE
    /// verb) so the host later surfaces its save UI instead of servicing it silently.
    fn alloc_fileref(&mut self, usage: u32, name: String, rock: u32, by_prompt: bool) -> u32 {
        let id = (self.filerefs.len() + 1) as u32;
        self.filerefs.push(Some(FileRef { id, rock, name, usage, by_prompt }));
        id
    }

    /// `glk_fileref_create_by_name`: sanitize `name`, allocate a fileref, return id.
    /// A game-fixed name (not `by_prompt`).
    pub fn fileref_create(&mut self, usage: u32, name: String, rock: u32) -> u32 {
        let name = Self::sanitize_fileref_name(&name);
        self.alloc_fileref(usage, name, rock, false)
    }

    /// Like [`Model::fileref_create`] but flagged `by_prompt` — the player picked
    /// the name via `glk_fileref_create_by_prompt` (their SAVE/RESTORE verb), so
    /// the host should surface its save UI for this fileref's `@save`/`@restore`.
    pub fn fileref_create_prompted(&mut self, usage: u32, name: String, rock: u32) -> u32 {
        let name = Self::sanitize_fileref_name(&name);
        self.alloc_fileref(usage, name, rock, true)
    }

    /// `glk_fileref_create_temp`: synthesize a unique name and allocate a fileref.
    pub fn fileref_create_temp(&mut self, usage: u32, rock: u32) -> u32 {
        let name = format!("__temp_{}__", self.filerefs.len());
        self.alloc_fileref(usage, name, rock, false)
    }

    /// `glk_fileref_create_by_prompt`: no TUI picker is available, so degrade to a
    /// fixed per-usage default name. (Known limitation.)
    pub fn fileref_create_by_prompt(&mut self, usage: u32, _fmode: u32, rock: u32) -> u32 {
        let name = format!("__prompt_{}__", usage & 0x0f);
        self.alloc_fileref(usage, name, rock, true)
    }

    /// The user-visible filenames in the VFS: every written file except the internal
    /// temp (`__temp_`) and legacy prompt (`__prompt_`) keys. BTreeMap order (sorted).
    /// Feeds the host's `create_by_prompt` read picker.
    pub fn file_names(&self) -> Vec<String> {
        self.files
            .keys()
            .filter(|k| !k.starts_with("__temp_") && !k.starts_with("__prompt_"))
            .cloned()
            .collect()
    }

    /// `glk_fileref_create_from_fileref`: clone `oldfref`'s name with a new
    /// usage/rock. Returns 0 if `oldfref` is invalid.
    pub fn fileref_create_from(&mut self, usage: u32, oldfref: u32, rock: u32) -> u32 {
        match self.fileref(oldfref).map(|f| f.name.clone()) {
            Some(name) => self.alloc_fileref(usage, name, rock, false),
            None => 0,
        }
    }

    /// `glk_fileref_destroy`: free the fileref slot. Does NOT delete the file.
    pub fn fileref_destroy(&mut self, fref: u32) {
        if fref != 0 {
            if let Some(slot) = self.filerefs.get_mut((fref - 1) as usize) {
                *slot = None;
            }
        }
    }

    /// A fileref's rock (0 if invalid).
    pub fn fileref_rock(&self, fref: u32) -> u32 {
        self.fileref(fref).map(|f| f.rock).unwrap_or(0)
    }

    /// Iterate filerefs: the smallest existing id greater than `fref` (`fref == 0`
    /// → the first), or `(0, 0)` when exhausted. Returns `(id, rock)`.
    pub fn fileref_iterate(&self, fref: u32) -> (u32, u32) {
        if let Some(f) = self.filerefs.iter().skip(fref as usize).flatten().next() {
            return (f.id, f.rock);
        }
        (0, 0)
    }

    /// `glk_fileref_does_file_exist`: the fileref is live AND its file has been
    /// written to the VFS — or, for a host-managed `SavedGame` slot, opened for
    /// writing this session (see [`Model::saved_game_files`]).
    pub fn fileref_exists(&self, fref: u32) -> bool {
        match self.fileref(fref) {
            Some(f) => self.files.contains_key(&f.name) || self.saved_game_files.contains_key(&f.name),
            None => false,
        }
    }

    /// `glk_fileref_delete_file`: remove the fileref's file from the VFS (and
    /// drop any host-managed `SavedGame` existence marker for its name).
    pub fn fileref_delete(&mut self, fref: u32) {
        if let Some(name) = self.fileref(fref).map(|f| f.name.clone()) {
            self.files.remove(&name);
            self.saved_game_files.remove(&name);
            self.vfs_dirty = true;
        }
    }

    /// The file VFS encoded as a standalone, self-describing sidecar blob (see
    /// [`encode_files`]) for on-disk persistence between sessions.
    pub fn vfs_bytes(&self) -> Vec<u8> {
        encode_files(&self.files)
    }

    /// Replace the file VFS from a sidecar blob (see [`decode_files`]). Called
    /// once at story-open before the game runs, so a full replace is correct; a
    /// corrupt/foreign blob decodes to an empty VFS.
    pub fn load_vfs(&mut self, bytes: &[u8]) {
        self.files = decode_files(bytes);
    }

    /// Seed a host-managed `SavedGame` slot's existence + size so a
    /// `create_by_name` game probing `glk_fileref_does_file_exist` before
    /// `@restore` sees its on-disk save across launches. `saved_game_files` is
    /// session-transient (host `.qzl` saves are decoupled from the VFS), so the
    /// host reseeds it from disk at boot, keyed by the raw fileref name. (SQ-0301)
    pub fn seed_saved_game_file(&mut self, name: String, size: u32) {
        self.saved_game_files.insert(name, size);
    }

    /// Whether the file VFS has been mutated since the last [`clear_vfs_dirty`].
    pub fn vfs_dirty(&self) -> bool {
        self.vfs_dirty
    }

    /// Clear the VFS-dirty flag (after a host flushes the sidecar, or after the
    /// initial load — loading is not a game mutation).
    pub fn clear_vfs_dirty(&mut self) {
        self.vfs_dirty = false;
    }

    /// The `(name, usage)` a fileref points at, for opening a file stream (Task 2).
    pub fn fileref_name(&self, fref: u32) -> Option<(String, u32)> {
        self.fileref(fref).map(|f| (f.name.clone(), f.usage))
    }

    // ── windows ───────────────────────────────────────────────────────────────

    /// Open a window. `split` is the window to split (0 for the first/root
    /// window). Returns the new window's id, or `None` on a malformed call.
    ///
    /// On a split a new [`WinType::Pair`] node replaces `split` in the tree, with
    /// `split` and the new window as its children. The new window is the key
    /// window for the split's sizing. Geometry is recomputed by the caller via
    /// [`Model::relayout`].
    pub fn window_open(&mut self, split: u32, method: u32, size: u32, wintype: u32, rock: u32) -> Option<u32> {
        let wt = WinType::from_arg(wintype)?;
        if wt == WinType::Pair {
            return None; // pair windows are created implicitly, never by request
        }
        let nid = self.alloc_window(wt, rock);
        let sid = self.alloc_stream(StreamKind::Window(nid), 0);
        self.win_mut(nid).unwrap().stream = sid;

        if split == 0 {
            if self.root != 0 {
                // A root already exists; opening a second root is malformed.
                self.free_window_subtree(nid);
                return None;
            }
            self.root = nid;
            return Some(nid);
        }

        if self.win(split).is_none() {
            self.free_window_subtree(nid);
            return None;
        }

        let pid = self.alloc_window(WinType::Pair, 0);
        let old_parent = self.win(split).unwrap().parent;
        {
            let p = self.win_mut(pid).unwrap();
            p.parent = old_parent;
            p.child1 = split;
            p.child2 = nid;
            p.key = nid;
            p.method = method;
            p.size = size;
        }
        self.win_mut(split).unwrap().parent = pid;
        self.win_mut(nid).unwrap().parent = pid;
        if old_parent == 0 {
            self.root = pid;
        } else {
            // Replace `split` with the new pair in its old parent's child links.
            let op = self.win_mut(old_parent).unwrap();
            if op.child1 == split {
                op.child1 = pid;
            } else if op.child2 == split {
                op.child2 = pid;
            }
        }
        Some(nid)
    }

    /// Close window `win` (and its whole subtree); collapse the parent pair so
    /// the sibling takes its place. Returns the closed window stream's
    /// `(read_count, write_count)`, or `None` if `win` is invalid.
    pub fn window_close(&mut self, win: u32) -> Option<(u32, u32)> {
        let w = self.win(win)?;
        let stream_id = w.stream;
        let counts = self
            .stream(stream_id)
            .map(|s| (s.read_count, s.write_count))
            .unwrap_or((0, 0));
        let parent = w.parent;

        if parent == 0 {
            // Closing the root empties the whole display.
            self.free_window_subtree(win);
            self.root = 0;
        } else {
            // The parent is a pair; promote the sibling into the pair's place.
            let pw = self.win(parent).unwrap();
            let sibling = if pw.child1 == win { pw.child2 } else { pw.child1 };
            let grandparent = pw.parent;
            self.free_window_subtree(win);
            self.win_mut(sibling).unwrap().parent = grandparent;
            if grandparent == 0 {
                self.root = sibling;
            } else {
                let gp = self.win_mut(grandparent).unwrap();
                if gp.child1 == parent {
                    gp.child1 = sibling;
                } else if gp.child2 == parent {
                    gp.child2 = sibling;
                }
            }
            // Free the now-defunct pair node.
            self.windows[(parent - 1) as usize] = None;
        }

        // A closed window's stream must not remain current.
        if self.cur_stream == stream_id {
            self.cur_stream = 0;
        }
        Some(counts)
    }

    /// Free `win` and all descendant windows, plus their streams.
    fn free_window_subtree(&mut self, win: u32) {
        let Some(w) = self.win(win) else { return };
        let (c1, c2, stream_id, is_pair) = (w.child1, w.child2, w.stream, w.wintype == WinType::Pair);
        if is_pair {
            self.free_window_subtree(c1);
            self.free_window_subtree(c2);
        }
        if stream_id != 0 {
            self.streams[(stream_id - 1) as usize] = None;
            if self.cur_stream == stream_id {
                self.cur_stream = 0;
            }
        }
        self.windows[(win - 1) as usize] = None;
    }

    /// Recompute every window's rectangle from the tree and `(width, height)`,
    /// returning the leaf-window layout `(id, type, rect, border)` in id order
    /// (to hand to [`GlkBackend::window_layout`]). `border` is the leaf's
    /// border-presence hint from its parent pair's split method (SQ-0286):
    /// `None` for a parentless root (no preference), `Some(false)` for a
    /// `winmethod_NoBorder` split, `Some(true)` for the default (`winmethod_Border`).
    pub fn relayout(&mut self, width: u32, height: u32, char_px: (u32, u32)) -> Vec<(u32, WinType, Rect, Option<bool>)> {
        self.char_px = char_px;
        // Snap the working screen size down so every proportional split lands on
        // whole cells (see `clean_dims`). Any leftover row/column is simply not
        // covered by a window — a harmless margin — rather than forcing a
        // fractional split a game's layout code may loop on.
        let (width, height) = self.clean_dims(width, height);
        if self.root != 0 {
            let r = Rect { left: 0, top: 0, width, height };
            self.layout_window(self.root, r);
        }
        let mut out = Vec::new();
        for w in self.windows.iter().flatten() {
            if w.wintype != WinType::Pair {
                // Border presence lives on the parent pair's split method; a
                // parentless root expresses no preference (`None`). (SQ-0286)
                let border = self
                    .window_parent(w.id)
                    .and_then(|p| self.window_arrangement(p))
                    .map(|(method, _, _)| (method & WINMETHOD_BORDERMASK) != WINMETHOD_NOBORDER);
                out.push((w.id, w.wintype, w.rect, border));
            }
        }
        out
    }

    /// Largest `(w, h) ≤ (width, height)` at which every **proportional** split in
    /// the window tree divides into whole cells, so no two siblings end up off by
    /// a rounding cell.
    ///
    /// Why: a terminal quantizes windows to character cells, so a 50 % split of an
    /// odd column count rounds to unequal halves (e.g. 40 | 39). Some games — an
    /// Inform 7 graphics/map sidebar among them — assume their proportional split
    /// is exact and spin forever on a fractional one, where a pixel interpreter's
    /// ≤1-pixel error is invisible. Snapping width/height independently (a
    /// Left/Right split constrains columns, an Above/Below split constrains rows)
    /// gives such splits the equal cells they expect. A non-halving ratio (e.g.
    /// 37 %) has no small clean size; there we fall back to the requested size and
    /// rely on the host's per-turn watchdog.
    fn clean_dims(&self, width: u32, height: u32) -> (u32, u32) {
        let w = (1..=width).rev().find(|&s| self.axis_splits_exact(self.root, s, true)).unwrap_or(width);
        let h = (1..=height).rev().find(|&s| self.axis_splits_exact(self.root, s, false)).unwrap_or(height);
        (w.max(1), h.max(1))
    }

    /// Does every proportional split that divides the given axis land on whole
    /// cells, if this subtree occupies `size` cells along that axis? `horizontal`
    /// selects the column axis (Left/Right splits) vs the row axis (Above/Below).
    fn axis_splits_exact(&self, id: u32, size: u32, horizontal: bool) -> bool {
        let w = match self.win(id) {
            Some(w) if w.wintype == WinType::Pair => w,
            _ => return true, // leaf or missing: nothing to constrain
        };
        let dir = w.method & WINMETHOD_DIRMASK;
        let vertical = dir == WINMETHOD_ABOVE || dir == WINMETHOD_BELOW;
        if vertical != horizontal {
            // This split divides the axis under test.
            let proportional = (w.method & WINMETHOD_DIVISIONMASK) == WINMETHOD_PROPORTIONAL;
            if proportional && !(size * w.size).is_multiple_of(100) {
                return false; // (total * pct) / 100 would truncate → fractional split
            }
            let new = if proportional {
                (size * w.size) / 100
            } else {
                let key_is_graphics = self.win(w.child2).map(|c| c.wintype) == Some(WinType::Graphics);
                if key_is_graphics {
                    let cell_px = if vertical { self.char_px.1 } else { self.char_px.0 }.max(1);
                    w.size.div_ceil(cell_px)
                } else {
                    w.size
                }
            }
            .min(size);
            self.axis_splits_exact(w.child1, size - new, horizontal)
                && self.axis_splits_exact(w.child2, new, horizontal)
        } else {
            // Splits the other axis: this axis passes full `size` to both children.
            self.axis_splits_exact(w.child1, size, horizontal)
                && self.axis_splits_exact(w.child2, size, horizontal)
        }
    }

    fn layout_window(&mut self, id: u32, rect: Rect) {
        let (wintype, method, size, child1, child2, key) = {
            let w = self.win_mut(id).unwrap();
            w.rect = rect;
            (w.wintype, w.method, w.size, w.child1, w.child2, w.key)
        };
        match wintype {
            WinType::TextGrid => {
                let w = self.win_mut(id).unwrap();
                w.grid.width = rect.width;
                w.grid.height = rect.height;
                if w.grid.cx >= rect.width {
                    w.grid.cx = 0;
                }
                if w.grid.cy >= rect.height {
                    w.grid.cy = 0;
                }
            }
            WinType::TextBuffer => {}
            WinType::Graphics => {}
            WinType::Pair => {
                let _ = key;
                // Graphics fixed-splits size in PIXELS; convert to whole cells
                // (rounding up so the requested pixels aren't clipped). The
                // window's *logical* pixel size reported to the game stays exactly
                // what it asked for — see `window_pixel_size` — so its layout math
                // isn't thrown off by the cell rounding.
                let key_is_graphics = self.win(child2).map(|w| w.wintype) == Some(WinType::Graphics);
                let is_fixed = (method & WINMETHOD_DIVISIONMASK) == WINMETHOD_FIXED;
                let eff_size = if key_is_graphics && is_fixed {
                    let dir = method & WINMETHOD_DIRMASK;
                    let vertical = dir == WINMETHOD_ABOVE || dir == WINMETHOD_BELOW;
                    let cell_px = if vertical { self.char_px.1 } else { self.char_px.0 }.max(1);
                    size.div_ceil(cell_px)
                } else {
                    size
                };
                let (r_old, r_new) = split_rect(rect, method, eff_size);
                self.layout_window(child1, r_old);
                self.layout_window(child2, r_new);
            }
        }
    }

    /// Root window id (0 if none).
    pub fn root(&self) -> u32 {
        self.root
    }
    /// A window's type, if it exists.
    pub fn window_type(&self, win: u32) -> Option<WinType> {
        self.win(win).map(|w| w.wintype)
    }
    /// A window's rock, if it exists.
    pub fn window_rock(&self, win: u32) -> Option<u32> {
        self.win(win).map(|w| w.rock)
    }
    /// A window's parent (0 = none / root). `None` if `win` is invalid.
    pub fn window_parent(&self, win: u32) -> Option<u32> {
        self.win(win).map(|w| w.parent)
    }
    /// A window's sibling within its parent pair (0 if it is the root).
    pub fn window_sibling(&self, win: u32) -> Option<u32> {
        let w = self.win(win)?;
        if w.parent == 0 {
            return Some(0);
        }
        let p = self.win(w.parent)?;
        Some(if p.child1 == win { p.child2 } else { p.child1 })
    }
    /// A window's `(width, height)` in characters. `None` if invalid.
    pub fn window_size(&self, win: u32) -> Option<(u32, u32)> {
        self.win(win).map(|w| (w.rect.width, w.rect.height))
    }
    /// Whether any open window is a graphics window (used to gate Redraw events
    /// on arrangement — text-only trees never need one).
    pub fn has_graphics_window(&self) -> bool {
        self.windows.iter().flatten().any(|w| w.wintype == WinType::Graphics)
    }
    /// A graphics window's `(width, height)` in PIXELS. Normally cells × char_px,
    /// but when the window is the key of a **fixed-pixel** split we report the
    /// exact pixels the game requested on the split axis rather than the
    /// cell-rounded value. A terminal can only allocate whole cells, so the
    /// footprint is rounded up (see `layout_window`) and the spare pixels are
    /// letterboxed — but the game drew its content for the pixel size it asked
    /// for, and reporting a rounded value here throws off layout code that
    /// assumes `get_size` echoes its request (an Inform 7 map sidebar spins
    /// forever on the mismatch). `None` if invalid or not a graphics window.
    pub fn window_pixel_size(&self, win: u32, char_px: (u32, u32)) -> Option<(u32, u32)> {
        let w = self.win(win)?;
        if w.wintype != WinType::Graphics {
            return None;
        }
        let mut px = (w.rect.width * char_px.0, w.rect.height * char_px.1);
        if let Some((method, size, keywin)) = self.window_parent(win).and_then(|p| self.window_arrangement(p)) {
            let is_fixed = (method & WINMETHOD_DIVISIONMASK) == WINMETHOD_FIXED;
            if is_fixed && keywin == win {
                let dir = method & WINMETHOD_DIRMASK;
                if dir == WINMETHOD_ABOVE || dir == WINMETHOD_BELOW {
                    px.1 = size.min(px.1); // fixed rows: exact requested height
                } else {
                    px.0 = size.min(px.0); // fixed cols: exact requested width
                }
            }
        }
        Some(px)
    }
    /// A window's own output stream id (0 for a pair window).
    pub fn window_stream(&self, win: u32) -> Option<u32> {
        self.win(win).map(|w| w.stream)
    }

    /// `glk_window_set_echo_stream`: set (or clear, with `str == 0`) the echo
    /// stream of `win`. Setting a window's echo to its own window stream is
    /// illegal (it would infinite-loop; Glk spec §3.6) — that request is refused
    /// (echo left cleared). Longer echo loops are additionally bounded at the
    /// output funnel in exec.rs. A no-op for an invalid or pair window.
    pub fn window_set_echo_stream(&mut self, win: u32, str: u32) {
        let own = self.win(win).map(|w| w.stream);
        if let Some(w) = self.win_mut(win) {
            w.echo = if str != 0 && Some(str) == own { 0 } else { str };
        }
    }

    /// `glk_window_get_echo_stream`: the window's echo stream id (0 = none, or
    /// an invalid/pair window).
    pub fn window_echo_stream(&self, win: u32) -> u32 {
        self.win(win).map(|w| w.echo).unwrap_or(0)
    }
    /// Iterate windows: the smallest existing id greater than `prev`
    /// (`prev == 0` → the first), or 0 when exhausted. Returns `(id, rock)`.
    pub fn window_iterate(&self, prev: u32) -> (u32, u32) {
        // ids are 1-based; slot index = id-1, so the next candidate is slot `prev`.
        if let Some(w) = self.windows.iter().skip(prev as usize).flatten().next() {
            return (w.id, w.rock);
        }
        (0, 0)
    }

    /// Set the grid cursor of a text-grid window (clamped to its bounds).
    pub fn window_move_cursor(&mut self, win: u32, x: u32, y: u32) {
        if let Some(w) = self.win_mut(win) {
            if w.wintype == WinType::TextGrid {
                w.grid.cx = x;
                w.grid.cy = y;
            }
        }
    }
    /// Clear a window: reset a grid's cursor to the origin. Returns the type so
    /// the caller can tell the backend which clear to issue.
    pub fn window_clear(&mut self, win: u32) -> Option<WinType> {
        let w = self.win_mut(win)?;
        if w.wintype == WinType::TextGrid {
            w.grid.cx = 0;
            w.grid.cy = 0;
        }
        Some(w.wintype)
    }

    /// Set/get a pair window's arrangement (the split method/size). Geometry is
    /// recomputed by the caller.
    pub fn window_set_arrangement(&mut self, win: u32, method: u32, size: u32, keywin: u32) {
        if let Some(w) = self.win_mut(win) {
            if w.wintype == WinType::Pair {
                w.method = method;
                w.size = size;
                if keywin != 0 {
                    w.key = keywin;
                }
            }
        }
    }
    /// A pair window's `(method, size, keywin)`. `None` if not a pair.
    pub fn window_arrangement(&self, win: u32) -> Option<(u32, u32, u32)> {
        let w = self.win(win)?;
        if w.wintype == WinType::Pair {
            Some((w.method, w.size, w.key))
        } else {
            None
        }
    }

    // ── grid cursor advance (for routing text-grid output) ────────────────────

    /// A text-grid window's `(width, height, cx, cy)`. `None` if not a grid.
    pub fn grid_state(&self, win: u32) -> Option<(u32, u32, u32, u32)> {
        let w = self.win(win)?;
        if w.wintype == WinType::TextGrid {
            Some((w.grid.width, w.grid.height, w.grid.cx, w.grid.cy))
        } else {
            None
        }
    }
    /// Set a text-grid window's cursor (no clamping; the router manages wrap).
    pub fn set_grid_cursor(&mut self, win: u32, cx: u32, cy: u32) {
        if let Some(w) = self.win_mut(win) {
            w.grid.cx = cx;
            w.grid.cy = cy;
        }
    }

    // ── streams ───────────────────────────────────────────────────────────────

    /// Open a Glulx-memory stream over `[addr, addr+len)` (in elements).
    /// `unicode` selects 32-bit elements. Returns the stream id.
    pub fn stream_open_memory(&mut self, addr: u32, len: u32, unicode: bool, rock: u32) -> u32 {
        self.alloc_stream(StreamKind::Memory { addr, len, pos: 0, unicode }, rock)
    }

    /// `glk_stream_open_file[_uni]`: open a file stream over the VFS blob named by
    /// `fref`. Applies the Glk open-mode rules — `Write` (0x01) truncates,
    /// `Read` (0x02) fails (returns 0, no stream) if the file is absent,
    /// `ReadWrite` (0x03) creates-if-absent at pos 0, `WriteAppend` (0x05)
    /// creates-if-absent and seeks to the end. `unicode` picks the on-file
    /// encoding. Returns the new stream id, or 0 on failure.
    pub fn stream_open_file(&mut self, fref: u32, fmode: u32, unicode: bool, rock: u32) -> u32 {
        let Some((name, usage)) = self.fileref_name(fref) else {
            return 0;
        };
        let by_prompt = self.fileref(fref).is_some_and(|f| f.by_prompt);
        // SavedGame-usage streams are host conduits, fully decoupled from the
        // VFS (saves live in host `.qzl` files, not `self.files`): no VFS entry
        // is created, and every mode succeeds — crucially Read succeeds even
        // with no prior save, so the game always reaches `@save`/`@restore`
        // and the host decides.
        if usage & 0x0f == 0x01 {
            // Opening in any create mode (Write / ReadWrite / WriteAppend) makes
            // the file "exist" — mirroring real Glk — so the game's post-`@save`
            // `glk_fileref_does_file_exist` check passes even though the save
            // bytes went to a host `.qzl`, not the VFS. Read (0x02) must NOT mark
            // existence: a game probing "is there a save?" before restoring still
            // sees false until something has actually saved. A create-mode open
            // truncates to length 0 until `@save` records the real byte count.
            match fmode {
                0x01 => {
                    self.saved_game_files.insert(name.clone(), 0);
                } // Write truncates
                0x03 | 0x05 => {
                    self.saved_game_files.entry(name.clone()).or_insert(0);
                }
                _ => {} // Read: leave existence/size as-is
            }
            let sid = self.alloc_stream(StreamKind::Null, rock);
            self.savegame_streams.insert(sid, SaveStream { name, pos: 0, by_prompt });
            return sid;
        }
        let pos = match fmode {
            0x01 => {
                self.files.insert(name.clone(), Vec::new());
                self.vfs_dirty = true;
                0
            }
            0x02 => {
                if !self.files.contains_key(&name) {
                    return 0;
                }
                0
            }
            0x03 => {
                self.files.entry(name.clone()).or_default();
                self.vfs_dirty = true;
                0
            }
            0x05 => {
                let len = self.files.entry(name.clone()).or_default().len();
                self.vfs_dirty = true;
                len
            }
            _ => return 0,
        };
        let sid = self.alloc_stream(StreamKind::File { unicode }, rock);
        self.file_streams.insert(sid, FileStream { name, mode: fmode, pos, unicode, usage, by_prompt });
        sid
    }

    /// `glk_stream_open_resource[_uni]`: open a read-only stream over a Blorb
    /// `Data` chunk whose bytes the host resolved (`data`) and whose chunk type
    /// it classified (`text` = a `TEXT` chunk, else `BINA`/`FORM` binary).
    /// `unicode` selects the `_uni` variant (32-bit read elements). Returns the
    /// new stream id. The caller returns 0 (open failed) when the host has no
    /// such resource, mirroring real Glk.
    pub fn stream_open_resource(&mut self, data: Vec<u8>, unicode: bool, text: bool, rock: u32) -> u32 {
        let sid = self.alloc_stream(StreamKind::Resource { unicode }, rock);
        self.resource_streams.insert(sid, ResourceStream { data, pos: 0, unicode, text });
        sid
    }

    /// `Some(unicode)` if `id` is a resource stream (its read-element width),
    /// else `None`. Lets the stream-read selectors branch onto the resource path
    /// exactly as [`Model::file_stream_is_unicode`] does for file streams.
    pub fn resource_stream_is_unicode(&self, id: u32) -> Option<bool> {
        match self.stream(id)?.kind {
            StreamKind::Resource { unicode } => Some(unicode),
            _ => None,
        }
    }

    /// Read one character from a resource stream at its cursor, decoding per
    /// the CHUNK type first (matching cheapglk `cgstream.c`): a `TEXT` chunk is
    /// UTF-8-decoded regardless of the stream's unicode-ness — a non-unicode
    /// read of a decoded code point above 0xFF clamps to `'?'` (cheapglk:
    /// `if (!want_unicode && ch >= 0x100) return '?';`), counting one read per
    /// decoded character. A binary (`BINA`/`FORM`) chunk reads raw bytes on a
    /// non-unicode stream and 4-byte big-endian words on a unicode one.
    /// Advances `pos` past the consumed bytes. `None` at end-of-file.
    pub fn resource_stream_read_char(&mut self, id: u32) -> Option<u32> {
        let rs = self.resource_streams.get(&id)?;
        let (val, adv) = if rs.text {
            let (cp, len) = Self::vfs_decode_utf8(&rs.data, rs.pos)?;
            let cp = if !rs.unicode && cp > 0xFF { b'?' as u32 } else { cp };
            (cp, len)
        } else if !rs.unicode {
            (*rs.data.get(rs.pos)? as u32, 1)
        } else {
            if rs.pos + 4 > rs.data.len() {
                return None;
            }
            let b = &rs.data[rs.pos..rs.pos + 4];
            (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), 4)
        };
        self.resource_streams.get_mut(&id).unwrap().pos += adv;
        if let Some(st) = self.stream_mut(id) {
            st.read_count = st.read_count.saturating_add(1);
        }
        Some(val)
    }
    /// Credit a stream's `write_count` by `n` without writing any bytes. Used
    /// by the Glulx `@save` shim: the real save bytes go to a host file (the
    /// stream the game opened stays empty), but the library checks the
    /// stream's write count to decide save succeeded, so this satisfies that
    /// check without embedding the save into the VFS. For a `SavedGame` stream
    /// it also records `n` as the slot's byte length (keyed by fileref name), so
    /// a later reopen-for-read + seek-to-end reports the true save size — the
    /// second half of the library's verification (CounterfeitMonkey's SAVE verb).
    pub fn note_stream_write(&mut self, id: u32, n: u32) {
        if let Some(st) = self.stream_mut(id) {
            st.write_count = st.write_count.saturating_add(n);
        }
        if let Some(ss) = self.savegame_streams.get(&id) {
            self.saved_game_files.insert(ss.name.clone(), n);
        }
    }

    /// The recorded byte length of a host-managed `SavedGame` slot by fileref
    /// name, or `None` if nothing has marked it this session. Lets `@save` tell a
    /// brand-new save (absent / 0) from a re-save over an existing slot, so a
    /// cancelled fresh save can be reverted to "absent" (SQ-0301).
    pub fn saved_game_size(&self, name: &str) -> Option<u32> {
        self.saved_game_files.get(name).copied()
    }

    /// Drop a host-managed `SavedGame` slot's existence marker by name. A
    /// cancelled/failed brand-new `@save` wrote no `.qzl`, so its open-for-write
    /// existence-at-0 must be undone, leaving `glk_fileref_does_file_exist`
    /// false (SQ-0301).
    pub fn clear_saved_game_file(&mut self, name: &str) {
        self.saved_game_files.remove(name);
    }

    /// For a live file-backed stream (a `SavedGame` `Null` conduit OR a VFS
    /// `File` stream — a game may `@save` to either), its `(fileref name,
    /// by_prompt)`. This is the hook `@save`/`@restore` use to route the game's
    /// own fixed-name saves (`create_by_name`) silently vs. surfacing the
    /// player's SAVE-verb UI (`create_by_prompt`). `None` for any other stream.
    pub fn stream_saveload_info(&self, id: u32) -> Option<(String, bool)> {
        if let Some(ss) = self.savegame_streams.get(&id) {
            return Some((ss.name.clone(), ss.by_prompt));
        }
        self.file_streams.get(&id).map(|fs| (fs.name.clone(), fs.by_prompt))
    }

    /// Close a stream, returning its `(read_count, write_count)`. The current
    /// stream is cleared if it was this one.
    pub fn stream_close(&mut self, id: u32) -> Option<(u32, u32)> {
        let counts = self.stream(id).map(|s| (s.read_count, s.write_count))?;
        // Window streams are owned by their window; only free memory/file streams
        // here. A file stream also drops its side-table cursor state.
        if let Some(kind) = self.stream(id).map(|s| s.kind) {
            if matches!(
                kind,
                StreamKind::Memory { .. } | StreamKind::File { .. } | StreamKind::Null | StreamKind::Resource { .. }
            ) {
                self.streams[(id - 1) as usize] = None;
            }
            if matches!(kind, StreamKind::File { .. }) {
                self.file_streams.remove(&id);
            }
            if matches!(kind, StreamKind::Null) {
                self.savegame_streams.remove(&id);
            }
            if matches!(kind, StreamKind::Resource { .. }) {
                self.resource_streams.remove(&id);
            }
        }
        if self.cur_stream == id {
            self.cur_stream = 0;
        }
        // A closed stream that was some window's echo stream: those windows stop
        // echoing (Glk spec §3.6).
        for w in self.windows.iter_mut().flatten() {
            if w.echo == id {
                w.echo = 0;
            }
        }
        Some(counts)
    }
    /// The current output stream (0 = none).
    pub fn current_stream(&self) -> u32 {
        self.cur_stream
    }
    /// Set the current output stream (0 = none).
    pub fn set_current_stream(&mut self, id: u32) {
        self.cur_stream = id;
    }
    /// A stream's rock, if it exists.
    pub fn stream_rock(&self, id: u32) -> Option<u32> {
        self.stream(id).map(|s| s.rock)
    }
    /// Iterate streams: smallest existing id greater than `prev`, with its rock.
    pub fn stream_iterate(&self, prev: u32) -> (u32, u32) {
        if let Some(s) = self.streams.iter().skip(prev as usize).flatten().next() {
            return (s.id, s.rock);
        }
        (0, 0)
    }
    /// A memory stream's current position (element index). `None` otherwise.
    pub fn stream_position(&self, id: u32) -> Option<u32> {
        match self.stream(id)?.kind {
            StreamKind::Memory { pos, .. } => Some(pos),
            StreamKind::File { .. } => Some(self.file_streams.get(&id).map_or(0, |f| f.pos as u32)),
            StreamKind::Window(_) => Some(self.stream(id)?.write_count),
            // A host-managed SavedGame stream reports its tracked cursor (the
            // library seeks to the end and reads the position to size the save).
            StreamKind::Null => Some(self.savegame_streams.get(&id).map_or(0, |ss| ss.pos)),
            StreamKind::Resource { .. } => {
                Some(self.resource_streams.get(&id).map_or(0, |rs| rs.pos as u32))
            }
        }
    }
    /// Seek a memory stream. `seekmode`: 0 = from start, 1 = from current,
    /// 2 = from end; clamped to `[0, len]`.
    pub fn stream_set_position(&mut self, id: u32, pos: i32, seekmode: u32) {
        match self.stream(id).map(|s| s.kind) {
            Some(StreamKind::Memory { .. }) => {
                if let Some(s) = self.stream_mut(id) {
                    if let StreamKind::Memory { len, pos: ref mut p, .. } = s.kind {
                        let base = match seekmode {
                            1 => *p as i64,
                            2 => len as i64,
                            _ => 0,
                        };
                        let np = (base + pos as i64).clamp(0, len as i64);
                        *p = np as u32;
                    }
                }
            }
            Some(StreamKind::File { .. }) => {
                if let Some(fs) = self.file_streams.get(&id) {
                    let len = self.files.get(&fs.name).map_or(0, |b| b.len()) as i64;
                    let base = match seekmode {
                        1 => fs.pos as i64,
                        2 => len,
                        _ => 0,
                    };
                    let np = (base + pos as i64).clamp(0, len) as usize;
                    self.file_streams.get_mut(&id).unwrap().pos = np;
                }
            }
            Some(StreamKind::Null) => {
                // Seek over the host-managed save's known byte length, so a
                // seek-to-end + get-position reports the true save size.
                if let Some(ss) = self.savegame_streams.get(&id) {
                    let len = self.saved_game_files.get(&ss.name).copied().unwrap_or(0) as i64;
                    let base = match seekmode {
                        1 => ss.pos as i64,
                        2 => len,
                        _ => 0,
                    };
                    let np = (base + pos as i64).clamp(0, len) as u32;
                    self.savegame_streams.get_mut(&id).unwrap().pos = np;
                }
            }
            Some(StreamKind::Resource { .. }) => {
                if let Some(rs) = self.resource_streams.get(&id) {
                    let len = rs.data.len() as i64;
                    let base = match seekmode {
                        1 => rs.pos as i64,
                        2 => len,
                        _ => 0,
                    };
                    let np = (base + pos as i64).clamp(0, len) as usize;
                    self.resource_streams.get_mut(&id).unwrap().pos = np;
                }
            }
            _ => {}
        }
    }

    /// The kind + current style + current hyperlink value of a stream, for
    /// output routing.
    pub fn stream_kind_style(&self, id: u32) -> Option<(StreamKind, GlkStyle, u32)> {
        self.stream(id).map(|s| (s.kind, s.style, s.link))
    }
    /// Set a stream's current style.
    pub fn set_stream_style(&mut self, id: u32, style: GlkStyle) {
        if let Some(s) = self.stream_mut(id) {
            s.style = style;
        }
    }
    /// Set a stream's current hyperlink value (`glk_set_hyperlink`); 0 clears it.
    pub fn set_stream_link(&mut self, id: u32, link: u32) {
        if let Some(s) = self.stream_mut(id) {
            s.link = link;
        }
    }
    /// Return `(addr, len, pos, unicode)` for a memory stream, or `None` if `id`
    /// is not a memory stream. Used by the stream-read selectors in exec.rs.
    pub fn memory_stream_read_info(&self, id: u32) -> Option<(u32, u32, u32, bool)> {
        match self.stream(id)?.kind {
            StreamKind::Memory { addr, len, pos, unicode } => Some((addr, len, pos, unicode)),
            _ => None,
        }
    }

    /// Advance a memory stream's read position by `n` elements and bump its
    /// `read_count`. Used by the stream-read selectors after consuming bytes.
    pub fn memory_stream_read_advance(&mut self, id: u32, n: u32) {
        if let Some(s) = self.stream_mut(id) {
            if let StreamKind::Memory { ref mut pos, .. } = s.kind {
                *pos = pos.saturating_add(n);
            }
            s.read_count = s.read_count.saturating_add(n);
        }
    }

    /// `Some(unicode)` if `id` is a file stream (the on-file encoding flag), else
    /// `None`. Lets the stream-read selectors branch onto the VFS path.
    pub fn file_stream_is_unicode(&self, id: u32) -> Option<bool> {
        match self.stream(id)?.kind {
            StreamKind::File { unicode } => Some(unicode),
            _ => None,
        }
    }

    /// Write `s` to a file stream, appending/overwriting bytes at its cursor in
    /// the VFS blob per the stream's encoding (byte = `ch & 0xFF`; unicode binary
    /// = 4-byte big-endian; unicode text = UTF-8), advancing `pos` and the write
    /// count. A no-op for a non-file / stale stream.
    pub fn file_stream_write(&mut self, id: u32, s: &str) {
        let Some(fs) = self.file_streams.get(&id) else {
            return;
        };
        let text = fs.usage & fileusage_TextMode != 0;
        let unicode = fs.unicode;
        let name = fs.name.clone();
        let mut pos = fs.pos;
        let Some(data) = self.files.get_mut(&name) else {
            return;
        };
        let mut nchars = 0u32;
        for ch in s.chars() {
            if !unicode {
                Self::vfs_write_byte(data, &mut pos, (ch as u32 & 0xFF) as u8);
            } else if !text {
                for b in (ch as u32).to_be_bytes() {
                    Self::vfs_write_byte(data, &mut pos, b);
                }
            } else {
                let mut buf = [0u8; 4];
                for &b in ch.encode_utf8(&mut buf).as_bytes() {
                    Self::vfs_write_byte(data, &mut pos, b);
                }
            }
            nchars += 1;
        }
        self.file_streams.get_mut(&id).unwrap().pos = pos;
        if let Some(st) = self.stream_mut(id) {
            st.write_count = st.write_count.saturating_add(nchars);
        }
        self.vfs_dirty = true;
    }

    /// Overwrite the byte at `*pos` (or push when at the end), then advance `*pos`.
    /// `*pos` is always `<= data.len()` (positions are clamped on seek), so a push
    /// never leaves a gap.
    fn vfs_write_byte(data: &mut Vec<u8>, pos: &mut usize, b: u8) {
        if *pos < data.len() {
            data[*pos] = b;
        } else {
            // `pos` can exceed `len` if another stream truncated this file while
            // this one held a cursor past the new end; zero-fill the gap so the
            // byte lands at the right offset (Glk leaves gap bytes undefined).
            data.resize(*pos, 0);
            data.push(b);
        }
        *pos += 1;
    }

    /// Read one character from a file stream at its cursor, decoding per the
    /// stream's encoding (byte / 4-byte-BE / UTF-8), advancing `pos` past the
    /// consumed bytes and bumping the read count. `None` at end-of-file.
    pub fn file_stream_read_char(&mut self, id: u32) -> Option<u32> {
        let fs = self.file_streams.get(&id)?;
        let text = fs.usage & fileusage_TextMode != 0;
        let unicode = fs.unicode;
        let pos = fs.pos;
        let name = fs.name.clone();
        let data = self.files.get(&name)?;
        let (val, adv) = if !unicode {
            (*data.get(pos)? as u32, 1)
        } else if !text {
            if pos + 4 > data.len() {
                return None;
            }
            let b = &data[pos..pos + 4];
            (u32::from_be_bytes([b[0], b[1], b[2], b[3]]), 4)
        } else {
            Self::vfs_decode_utf8(data, pos)?
        };
        self.file_streams.get_mut(&id).unwrap().pos = pos + adv;
        if let Some(st) = self.stream_mut(id) {
            st.read_count = st.read_count.saturating_add(1);
        }
        Some(val)
    }

    /// Decode one UTF-8 code point from `data` at `pos`, returning `(codepoint,
    /// byte_len)`. `None` at EOF or on a truncated/invalid sequence.
    fn vfs_decode_utf8(data: &[u8], pos: usize) -> Option<(u32, usize)> {
        let b0 = *data.get(pos)?;
        let len = if b0 < 0x80 {
            1
        } else if b0 >> 5 == 0b110 {
            2
        } else if b0 >> 4 == 0b1110 {
            3
        } else if b0 >> 3 == 0b11110 {
            4
        } else {
            return None;
        };
        if pos + len > data.len() {
            return None;
        }
        let ch = std::str::from_utf8(&data[pos..pos + len]).ok()?.chars().next()?;
        Some((ch as u32, len))
    }

    /// Advance a memory stream's position by `n` elements (after the engine has
    /// written the bytes), bumping the write count.
    pub fn memory_stream_advance(&mut self, id: u32, n: u32) {
        if let Some(s) = self.stream_mut(id) {
            if let StreamKind::Memory { ref mut pos, .. } = s.kind {
                *pos = pos.saturating_add(n);
            }
            s.write_count = s.write_count.saturating_add(n);
        }
    }
    /// Bump a window stream's write count by `n` characters.
    pub fn window_stream_advance(&mut self, id: u32, n: u32) {
        if let Some(s) = self.stream_mut(id) {
            s.write_count = s.write_count.saturating_add(n);
        }
    }

    // ── input requests (3a-2) ─────────────────────────────────────────────────

    /// Record a pending line-input request on `win` (replacing any prior line or
    /// char request on it). Returns `false` for a non-existent or pair window.
    pub fn request_line_event(&mut self, win: u32, buf: u32, maxlen: u32, initlen: u32, unicode: bool) -> bool {
        match self.win_mut(win) {
            Some(w) if w.wintype != WinType::Pair => {
                w.char_req = None;
                w.line_req = Some(LineReq { buf, maxlen, initlen: initlen.min(maxlen), unicode });
                true
            }
            _ => false,
        }
    }

    /// Record a pending char-input request on `win` (replacing any prior request
    /// on it). Returns `false` for a non-existent or pair window.
    pub fn request_char_event(&mut self, win: u32, unicode: bool) -> bool {
        match self.win_mut(win) {
            Some(w) if w.wintype != WinType::Pair => {
                w.line_req = None;
                w.char_req = Some(CharReq { unicode });
                true
            }
            _ => false,
        }
    }

    /// The pending line request on `win`, if any.
    pub fn line_request(&self, win: u32) -> Option<LineReq> {
        self.win(win).and_then(|w| w.line_req)
    }
    /// The pending char request on `win`, if any.
    pub fn char_request(&self, win: u32) -> Option<CharReq> {
        self.win(win).and_then(|w| w.char_req)
    }

    /// Take (and clear) the pending line request on `win`.
    pub fn take_line_request(&mut self, win: u32) -> Option<LineReq> {
        self.win_mut(win).and_then(|w| w.line_req.take())
    }
    /// Take (and clear) the pending char request on `win`.
    pub fn take_char_request(&mut self, win: u32) -> Option<CharReq> {
        self.win_mut(win).and_then(|w| w.char_req.take())
    }

    /// Arm a mouse-input request on `win` (`glk_request_mouse_event`). A no-op
    /// on a non-existent or pair window.
    pub fn set_mouse_request(&mut self, win: u32) {
        if let Some(w) = self.win_mut(win) {
            if w.wintype != WinType::Pair {
                w.mouse_req = true;
            }
        }
    }
    /// Take (and clear) the pending mouse request on `win`, returning whether one
    /// was armed. Used both to gate and to consume delivery — a Glk mouse event
    /// is one-shot, so the request clears the moment it fires.
    pub fn take_mouse_request(&mut self, win: u32) -> bool {
        match self.win_mut(win) {
            Some(w) => std::mem::take(&mut w.mouse_req),
            None => false,
        }
    }
    /// Whether `win` currently has a pending mouse request.
    pub fn mouse_requested(&self, win: u32) -> bool {
        self.win(win).map(|w| w.mouse_req).unwrap_or(false)
    }
    /// Arm a hyperlink-input request on `win` (`glk_request_hyperlink_event`).
    /// A no-op on a non-existent or pair window.
    pub fn set_hyperlink_request(&mut self, win: u32) {
        if let Some(w) = self.win_mut(win) {
            if w.wintype != WinType::Pair {
                w.hyperlink_req = true;
            }
        }
    }
    /// Take (and clear) the pending hyperlink request on `win`, returning whether
    /// one was armed. Like a mouse event a Glk hyperlink event is one-shot, so the
    /// request clears the moment it fires.
    pub fn take_hyperlink_request(&mut self, win: u32) -> bool {
        match self.win_mut(win) {
            Some(w) => std::mem::take(&mut w.hyperlink_req),
            None => false,
        }
    }
    /// Whether `win` currently has a pending hyperlink request.
    pub fn hyperlink_requested(&self, win: u32) -> bool {
        self.win(win).map(|w| w.hyperlink_req).unwrap_or(false)
    }

    /// Whether any window has a pending mouse request. The VM reads this at a
    /// `glk_select` with no line/char request to decide whether to block on a
    /// non-input event (see [`crate::exec::StepResult::NeedEvent`]).
    pub fn any_mouse_requested(&self) -> bool {
        self.windows.iter().flatten().any(|w| w.mouse_req)
    }

    /// Whether any window has a pending hyperlink request. Companion to
    /// [`Model::any_mouse_requested`] for the non-input `glk_select` case.
    pub fn any_hyperlink_requested(&self) -> bool {
        self.windows.iter().flatten().any(|w| w.hyperlink_req)
    }

    /// Every window (lowest id first) with a pending mouse request, as
    /// `(id, wintype, rect)`. The host reads this to decide whether a terminal
    /// click lands inside a mouse-watching window.
    pub fn mouse_windows(&self) -> Vec<(u32, WinType, Rect)> {
        self.windows
            .iter()
            .flatten()
            .filter(|w| w.mouse_req)
            .map(|w| (w.id, w.wintype, w.rect))
            .collect()
    }

    /// Record the line-input terminator keys for `win`
    /// (`glk_set_terminators_line_event`). Invalid keycodes are silently dropped
    /// (see [`keycode::is_terminator`]); an empty set restores Enter-only. The
    /// set persists across line requests. Returns `false` for a non-existent or
    /// pair window.
    pub fn set_line_terminators(&mut self, win: u32, keys: &[u32]) -> bool {
        match self.win_mut(win) {
            Some(w) if w.wintype != WinType::Pair => {
                w.terminators = keys.iter().copied().filter(|&k| keycode::is_terminator(k)).collect();
                true
            }
            _ => false,
        }
    }

    /// Whether `key` is an active line-input terminator for `win`.
    pub fn is_line_terminator(&self, win: u32, key: u32) -> bool {
        self.win(win).map(|w| w.terminators.contains(&key)).unwrap_or(false)
    }

    /// Set the window's line-echo flag (`glk_set_echo_line_event`). No-op for an
    /// invalid window id.
    pub fn set_window_echo_line(&mut self, win: u32, on: bool) {
        if let Some(w) = self.win_mut(win) {
            w.echo_line = on;
        }
    }

    /// The window's line-echo flag (default true; unknown window → true).
    pub fn window_echo_line(&self, win: u32) -> bool {
        self.win(win).map(|w| w.echo_line).unwrap_or(true)
    }

    /// The resolved colour for `GlkStyle::Input` in this window (its wintype's
    /// Input style hint); falls back to a text-buffer Input colour for an unknown
    /// window. Used by scrolling-terminal hosts to colour the input echo.
    pub fn window_input_colour(&self, win: u32) -> StyleColour {
        let wt = self.win(win).map(|w| w.wintype).unwrap_or(WinType::TextBuffer);
        self.style_colour(wt, GlkStyle::Input)
    }

    /// The first window (lowest id) with a pending line request: `(win, unicode)`.
    pub fn first_line_request(&self) -> Option<(u32, bool)> {
        self.windows
            .iter()
            .flatten()
            .find_map(|w| w.line_req.map(|r| (w.id, r.unicode)))
    }
    /// The first window (lowest id) with a pending char request: `(win, unicode)`.
    pub fn first_char_request(&self) -> Option<(u32, bool)> {
        self.windows
            .iter()
            .flatten()
            .find_map(|w| w.char_req.map(|r| (w.id, r.unicode)))
    }

    // ── event queue (arrange/redraw; input events are delivered directly) ─────

    /// Queue a non-input event for the next `glk_select`. Arrange/Redraw events
    /// dedupe (one of each suffices until consumed).
    pub fn push_event(&mut self, ev: GlkEvent) {
        if (ev.etype == evtype::ARRANGE || ev.etype == evtype::REDRAW)
            && self.events.iter().any(|e| e.etype == ev.etype && e.win == ev.win)
        {
            return;
        }
        self.events.push_back(ev);
    }
    /// Pop the next queued non-input event, if any.
    pub fn pop_event(&mut self) -> Option<GlkEvent> {
        self.events.pop_front()
    }
    /// Drain all queued non-input events (test accessor).
    pub fn take_pending_events(&mut self) -> Vec<GlkEvent> {
        self.events.drain(..).collect()
    }

    // ── snapshot serialization (the Glulx-Quetzal "Glk " chunk; GLULX_NOTES §20) ──

    /// Serialize the model's structural state (window tree, streams, root +
    /// current stream) as the body of a `Glk ` save chunk. All fields are
    /// 32-bit big-endian, matching the rest of the save format. Text-grid CELL
    /// glyphs and text-buffer scrollback live in the backend (re-rendered by the
    /// host on restore) and are intentionally not serialized; only the grid
    /// dimensions + cursor are. The transient event queue is not serialized.
    pub(crate) fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let w = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_be_bytes());
        w(&mut out, GLK_SNAPSHOT_VERSION);
        w(&mut out, self.root);
        w(&mut out, self.cur_stream);
        w(&mut out, self.windows.len() as u32);
        for slot in &self.windows {
            match slot {
                None => w(&mut out, 0),
                Some(win) => {
                    w(&mut out, 1);
                    w(&mut out, win.id);
                    w(&mut out, win.wintype.to_arg());
                    w(&mut out, win.rock);
                    w(&mut out, win.parent);
                    w(&mut out, win.stream);
                    w(&mut out, win.child1);
                    w(&mut out, win.child2);
                    w(&mut out, win.key);
                    w(&mut out, win.method);
                    w(&mut out, win.size);
                    w(&mut out, win.rect.left);
                    w(&mut out, win.rect.top);
                    w(&mut out, win.rect.width);
                    w(&mut out, win.rect.height);
                    w(&mut out, win.grid.width);
                    w(&mut out, win.grid.height);
                    w(&mut out, win.grid.cx);
                    w(&mut out, win.grid.cy);
                    match win.line_req {
                        None => w(&mut out, 0),
                        Some(lr) => {
                            w(&mut out, 1);
                            w(&mut out, lr.buf);
                            w(&mut out, lr.maxlen);
                            w(&mut out, lr.initlen);
                            w(&mut out, lr.unicode as u32);
                        }
                    }
                    match win.char_req {
                        None => w(&mut out, 0),
                        Some(cr) => {
                            w(&mut out, 1);
                            w(&mut out, cr.unicode as u32);
                        }
                    }
                    w(&mut out, win.mouse_req as u32);
                    w(&mut out, win.hyperlink_req as u32);
                }
            }
        }
        w(&mut out, self.streams.len() as u32);
        for slot in &self.streams {
            match slot {
                None => w(&mut out, 0),
                Some(s) => {
                    w(&mut out, 1);
                    w(&mut out, s.id);
                    w(&mut out, s.rock);
                    w(&mut out, s.style.to_num());
                    w(&mut out, s.link);
                    w(&mut out, s.read_count);
                    w(&mut out, s.write_count);
                    match s.kind {
                        StreamKind::Window(win) => {
                            w(&mut out, 0);
                            w(&mut out, win);
                        }
                        StreamKind::Memory { addr, len, pos, unicode } => {
                            w(&mut out, 1);
                            w(&mut out, addr);
                            w(&mut out, len);
                            w(&mut out, pos);
                            w(&mut out, unicode as u32);
                        }
                        StreamKind::File { unicode } => {
                            // The mutable cursor (name/mode/pos/usage) is persisted
                            // out of band below, keyed by stream id, in the
                            // `file_streams` table; here we only tag the kind and
                            // its encoding so the stream slot reconstructs.
                            w(&mut out, 2);
                            w(&mut out, unicode as u32);
                        }
                        StreamKind::Null => {
                            w(&mut out, 3);
                        }
                        StreamKind::Resource { unicode } => {
                            // The bytes + cursor + text flag persist out of band
                            // below in the `resource_streams` table, keyed by
                            // stream id; here we only tag the kind + encoding.
                            w(&mut out, 4);
                            w(&mut out, unicode as u32);
                        }
                    }
                }
            }
        }
        // Snapshot version 4: persist the in-memory VFS so file streams and
        // filerefs round-trip. Names and byte blobs are length-prefixed (u32 len
        // then the raw bytes).
        let wb = |out: &mut Vec<u8>, b: &[u8]| {
            out.extend_from_slice(&(b.len() as u32).to_be_bytes());
            out.extend_from_slice(b);
        };
        // (a) files: name → contents.
        w(&mut out, self.files.len() as u32);
        for (name, blob) in &self.files {
            wb(&mut out, name.as_bytes());
            wb(&mut out, blob);
        }
        // (b) filerefs table: presence-flagged per slot so ids stay stable across
        // freed (None) slots, mirroring the window/stream loops above.
        w(&mut out, self.filerefs.len() as u32);
        for slot in &self.filerefs {
            match slot {
                None => w(&mut out, 0),
                Some(fr) => {
                    w(&mut out, 1);
                    w(&mut out, fr.id);
                    w(&mut out, fr.rock);
                    w(&mut out, fr.usage);
                    wb(&mut out, fr.name.as_bytes());
                }
            }
        }
        // (c) file_streams cursor side table, keyed by stream id.
        w(&mut out, self.file_streams.len() as u32);
        for (sid, fs) in &self.file_streams {
            w(&mut out, *sid);
            wb(&mut out, fs.name.as_bytes());
            w(&mut out, fs.mode);
            w(&mut out, fs.pos as u32);
            w(&mut out, fs.unicode as u32);
            w(&mut out, fs.usage);
        }
        // (d) resource_streams side table (snapshot version 5): the owned bytes
        // are re-derivable from the host Blorb, but the model can't reach the
        // host at restore, so an open resource stream round-trips its bytes here
        // (mirroring the file_streams precedent) — a game holding one open across
        // a host snapshot keeps reading from where it left off.
        w(&mut out, self.resource_streams.len() as u32);
        for (sid, rs) in &self.resource_streams {
            w(&mut out, *sid);
            wb(&mut out, &rs.data);
            w(&mut out, rs.pos as u32);
            w(&mut out, rs.unicode as u32);
            w(&mut out, rs.text as u32);
        }
        out
    }

    /// Rebuild a model from a `Glk ` chunk body (see [`Model::serialize`]). Never
    /// panics: any truncation, bad enum value, or trailing garbage is reported as
    /// an error string (mapped to `GError::BadSave` by the caller). Slot vectors
    /// are grown by pushing, so a corrupt count cannot trigger a huge allocation.
    pub(crate) fn deserialize(data: &[u8]) -> Result<Model, String> {
        let mut r = SnapReader::new(data);
        let version = r.u32()?;
        // Version 4 snapshots (pre resource streams, SQ-0308) lack the trailing
        // resource_streams table and stream-kind tag 4; still restorable.
        if version != 4 && version != GLK_SNAPSHOT_VERSION {
            return Err(format!("unsupported Glk snapshot version {version}"));
        }
        let root = r.u32()?;
        let cur_stream = r.u32()?;

        let nwin = r.u32()?;
        let mut windows = Vec::new();
        for _ in 0..nwin {
            if r.u32()? == 0 {
                windows.push(None);
                continue;
            }
            let id = r.u32()?;
            let wintype = WinType::from_arg(r.u32()?).ok_or("Glk snapshot: bad window type")?;
            let rock = r.u32()?;
            let parent = r.u32()?;
            let stream = r.u32()?;
            let child1 = r.u32()?;
            let child2 = r.u32()?;
            let key = r.u32()?;
            let method = r.u32()?;
            let size = r.u32()?;
            let rect = Rect { left: r.u32()?, top: r.u32()?, width: r.u32()?, height: r.u32()? };
            let grid = Grid { width: r.u32()?, height: r.u32()?, cx: r.u32()?, cy: r.u32()? };
            let line_req = if r.u32()? != 0 {
                Some(LineReq { buf: r.u32()?, maxlen: r.u32()?, initlen: r.u32()?, unicode: r.u32()? != 0 })
            } else {
                None
            };
            let char_req = if r.u32()? != 0 { Some(CharReq { unicode: r.u32()? != 0 }) } else { None };
            let mouse_req = r.u32()? != 0;
            let hyperlink_req = r.u32()? != 0;
            windows.push(Some(Window {
                id, wintype, rock, parent, stream, rect, grid, line_req, char_req, mouse_req,
                hyperlink_req, terminators: Vec::new(), echo_line: true, echo: 0, child1, child2, key, method, size,
            }));
        }

        let nstream = r.u32()?;
        let mut streams = Vec::new();
        for _ in 0..nstream {
            if r.u32()? == 0 {
                streams.push(None);
                continue;
            }
            let id = r.u32()?;
            let rock = r.u32()?;
            let style = GlkStyle::from_num(r.u32()?);
            let link = r.u32()?;
            let read_count = r.u32()?;
            let write_count = r.u32()?;
            let kind = match r.u32()? {
                0 => StreamKind::Window(r.u32()?),
                1 => StreamKind::Memory { addr: r.u32()?, len: r.u32()?, pos: r.u32()?, unicode: r.u32()? != 0 },
                // The cursor (name/mode/pos/usage) is restored from the
                // `file_streams` table below, keyed by this stream's id.
                2 => StreamKind::File { unicode: r.u32()? != 0 },
                3 => StreamKind::Null,
                // The bytes/cursor/text flag are restored from the
                // `resource_streams` table below, keyed by this stream's id.
                4 => StreamKind::Resource { unicode: r.u32()? != 0 },
                other => return Err(format!("Glk snapshot: bad stream kind {other}")),
            };
            streams.push(Some(Stream {
                id,
                rock,
                kind,
                style,
                link,
                zfg: None,
                zbg: None,
                zrev: None,
                read_count,
                write_count,
            }));
        }

        // Version 4: the in-memory VFS — files, filerefs, and the file-stream cursors.
        let mut files = std::collections::BTreeMap::new();
        let nfiles = r.u32()?;
        for _ in 0..nfiles {
            let name = r.string()?;
            let blob = r.bytes()?;
            files.insert(name, blob);
        }
        let mut filerefs = Vec::new();
        let nfref = r.u32()?;
        for _ in 0..nfref {
            if r.u32()? == 0 {
                filerefs.push(None);
                continue;
            }
            let id = r.u32()?;
            let rock = r.u32()?;
            let usage = r.u32()?;
            let name = r.string()?;
            filerefs.push(Some(FileRef { id, rock, name, usage, by_prompt: false }));
        }
        let mut file_streams = std::collections::BTreeMap::new();
        let nfs = r.u32()?;
        for _ in 0..nfs {
            let sid = r.u32()?;
            let name = r.string()?;
            let mode = r.u32()?;
            let pos = r.u32()? as usize;
            let unicode = r.u32()? != 0;
            let usage = r.u32()?;
            file_streams.insert(sid, FileStream { name, mode, pos, unicode, usage, by_prompt: false });
        }
        // (d) resource_streams side table (snapshot version 5; absent in v4).
        let mut resource_streams = std::collections::BTreeMap::new();
        if version >= 5 {
            let nrs = r.u32()?;
            for _ in 0..nrs {
                let sid = r.u32()?;
                let data = r.bytes()?;
                let pos = r.u32()? as usize;
                let unicode = r.u32()? != 0;
                let text = r.u32()? != 0;
                resource_streams.insert(sid, ResourceStream { data, pos, unicode, text });
            }
        }

        if !r.done() {
            return Err("Glk snapshot: trailing bytes".to_string());
        }
        Ok(Model {
            windows,
            streams,
            files,
            filerefs,
            file_streams,
            root,
            cur_stream,
            events: std::collections::VecDeque::new(),
            style_hints: [[StyleColour::default(); NUMSTYLES as usize]; 2],
            style_attrs: [[StyleAttrs::default(); NUMSTYLES as usize]; 2],
            char_px: (1, 1),
            vfs_dirty: false,
            saved_game_files: std::collections::BTreeMap::new(),
            savegame_streams: std::collections::BTreeMap::new(),
            resource_streams,
        })
    }
}

/// Serialize the file VFS (name → bytes) as a standalone sidecar blob for
/// on-disk persistence: magic `GVFS` + `u32` version (1) + `u32` count, then
/// per entry a length-prefixed name and a length-prefixed blob (all lengths
/// big-endian `u32`). Keys beginning with `__temp_` are session-scoped Glk temp
/// files and are skipped. This is a superset (magic+version header) of the
/// files block inside the Glk save snapshot; the two are deliberately kept
/// separate so the snapshot wire format never depends on this header.
pub fn encode_files(files: &std::collections::BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"GVFS");
    out.extend_from_slice(&1u32.to_be_bytes());
    let entries: Vec<(&String, &Vec<u8>)> =
        files.iter().filter(|(name, _)| !name.starts_with("__temp_")).collect();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    let wb = |out: &mut Vec<u8>, b: &[u8]| {
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        out.extend_from_slice(b);
    };
    for (name, blob) in entries {
        wb(&mut out, name.as_bytes());
        wb(&mut out, blob);
    }
    out
}

/// Inverse of [`encode_files`], fully tolerant of corruption: a wrong magic,
/// unknown version, invalid UTF-8 name, or any truncation yields an empty map
/// (never a panic or error), so a foreign or damaged sidecar simply starts the
/// game from an empty VFS.
pub fn decode_files(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    decode_files_inner(bytes).unwrap_or_default()
}

fn decode_files_inner(bytes: &[u8]) -> Option<std::collections::BTreeMap<String, Vec<u8>>> {
    let mut p = 0usize;
    if bytes.get(0..4)? != b"GVFS" {
        return None;
    }
    p += 4;
    if read_u32(bytes, &mut p)? != 1 {
        return None; // unknown version
    }
    let count = read_u32(bytes, &mut p)?;
    let mut map = std::collections::BTreeMap::new();
    for _ in 0..count {
        let name = String::from_utf8(read_blob(bytes, &mut p)?).ok()?;
        let blob = read_blob(bytes, &mut p)?;
        map.insert(name, blob);
    }
    Some(map)
}

/// Read a big-endian `u32` at `*p`, advancing it; `None` on underflow.
fn read_u32(bytes: &[u8], p: &mut usize) -> Option<u32> {
    let end = p.checked_add(4)?;
    let s = bytes.get(*p..end)?;
    *p = end;
    Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Read a `u32`-length-prefixed byte blob at `*p`, advancing it; `None` on
/// underflow.
fn read_blob(bytes: &[u8], p: &mut usize) -> Option<Vec<u8>> {
    let len = read_u32(bytes, p)? as usize;
    let end = p.checked_add(len)?;
    let s = bytes.get(*p..end)?;
    *p = end;
    Some(s.to_vec())
}

/// Version tag at the head of a `Glk ` snapshot chunk (bumped on a format change).
const GLK_SNAPSHOT_VERSION: u32 = 5;

/// Sequential big-endian-`u32` reader over a `Glk ` snapshot chunk. Underflow is
/// an error, never a panic.
struct SnapReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> SnapReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        SnapReader { data, pos: 0 }
    }
    fn u32(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.data.len() {
            return Err("Glk snapshot: truncated".to_string());
        }
        let b = &self.data[self.pos..self.pos + 4];
        self.pos += 4;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// Read a `u32` length prefix then that many raw bytes. Underflow (a length
    /// exceeding the remaining input) is an error, never a panic.
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let len = self.u32()? as usize;
        if self.pos + len > self.data.len() {
            return Err("Glk snapshot: truncated blob".to_string());
        }
        let b = self.data[self.pos..self.pos + len].to_vec();
        self.pos += len;
        Ok(b)
    }
    /// Read a length-prefixed UTF-8 string (a fileref/file name).
    fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?).map_err(|_| "Glk snapshot: invalid UTF-8 name".to_string())
    }
    fn done(&self) -> bool {
        self.pos == self.data.len()
    }
}

/// Split `rect` into `(rect_for_old_window, rect_for_new_window)` per a
/// `winmethod`. The new (key) window is placed on the side named by the
/// direction; its size is `size` characters (Fixed) or `size`% (Proportional)
/// of the split axis. An oversized request collapses the old window to zero
/// (Glk spec §3.3: undersized windows get zero, not renegotiation).
fn split_rect(rect: Rect, method: u32, size: u32) -> (Rect, Rect) {
    let dir = method & WINMETHOD_DIRMASK;
    let division = method & WINMETHOD_DIVISIONMASK;
    let vertical = dir == WINMETHOD_ABOVE || dir == WINMETHOD_BELOW;
    let total = if vertical { rect.height } else { rect.width };
    let new_size = if division == WINMETHOD_PROPORTIONAL {
        (total * size) / 100
    } else {
        size
    }
    .min(total);
    let old_size = total - new_size;

    match dir {
        WINMETHOD_LEFT => (
            Rect { left: rect.left + new_size, width: old_size, ..rect },
            Rect { left: rect.left, width: new_size, ..rect },
        ),
        WINMETHOD_RIGHT => (
            Rect { left: rect.left, width: old_size, ..rect },
            Rect { left: rect.left + old_size, width: new_size, ..rect },
        ),
        WINMETHOD_ABOVE => (
            Rect { top: rect.top + new_size, height: old_size, ..rect },
            Rect { top: rect.top, height: new_size, ..rect },
        ),
        // WINMETHOD_BELOW (and any unknown direction defaults to below).
        _ => (
            Rect { top: rect.top, height: old_size, ..rect },
            Rect { top: rect.top + old_size, height: new_size, ..rect },
        ),
    }
}

#[cfg(test)]
mod layout_snap_tests {
    use super::*;

    // Left|Proportional 50% sidebar (like an Inform 7 map): an odd column count
    // must snap down so the two halves are equal cells, not 41|40.
    #[test]
    fn relayout_snaps_odd_width_so_proportional_halves_are_equal() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root
        let gfx = m.window_open(buf, WINMETHOD_LEFT | WINMETHOD_PROPORTIONAL, 50, 5, 0).unwrap();
        m.relayout(81, 41, (9, 19));
        let gw = m.window_size(gfx).unwrap().0;
        let bw = m.window_size(buf).unwrap().0;
        assert_eq!(gw, bw, "50% split must be equal halves (gfx={gw}, buf={bw})");
        assert_eq!(gw + bw, 80, "snapped to the largest even width ≤ 81");
    }

    #[test]
    fn relayout_leaves_even_width_untouched() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let gfx = m.window_open(buf, WINMETHOD_LEFT | WINMETHOD_PROPORTIONAL, 50, 5, 0).unwrap();
        m.relayout(80, 40, (9, 19));
        assert_eq!(m.window_size(gfx).unwrap().0, 40);
        assert_eq!(m.window_size(buf).unwrap().0, 40);
    }

    // A vertical proportional split constrains rows, not columns.
    #[test]
    fn relayout_snaps_odd_height_for_vertical_proportional_split() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let top = m.window_open(buf, WINMETHOD_ABOVE | WINMETHOD_PROPORTIONAL, 50, 5, 0).unwrap();
        m.relayout(80, 41, (9, 19));
        assert_eq!(m.window_size(top).unwrap().1, m.window_size(buf).unwrap().1, "equal rows");
        assert_eq!(m.window_size(top).unwrap().1 + m.window_size(buf).unwrap().1, 40, "snapped 41→40 rows");
        // Width (no horizontal proportional split) is untouched.
        assert_eq!(m.window_size(top).unwrap().0, 80);
    }

    // A fixed-pixel graphics sidebar (e.g. an Inform 7 map at its max size):
    // the terminal footprint rounds up to whole cells, but glk_window_get_size
    // must report the exact pixels the game requested — otherwise layout code
    // that assumes get_size echoes its request loops forever on the mismatch.
    #[test]
    fn fixed_graphics_split_reports_exact_requested_pixels() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        // Fixed Left 722px sidebar; 722/9 = 80.2 → 81-cell footprint (729px).
        let gfx = m.window_open(buf, WINMETHOD_LEFT | WINMETHOD_FIXED, 722, 5, 0).unwrap();
        m.relayout(200, 48, (9, 19));
        assert_eq!(m.window_size(gfx).unwrap().0, 81, "footprint rounds up to whole cells");
        let (pw, ph) = m.window_pixel_size(gfx, (9, 19)).unwrap();
        assert_eq!(pw, 722, "reports the exact requested width, not 81×9=729");
        // The non-fixed axis stays cells × char_px.
        assert_eq!(ph, m.window_size(gfx).unwrap().1 * 19, "height still cells × char_px");
    }

    // A fixed split imposes no proportional constraint: an odd screen passes through.
    #[test]
    fn relayout_does_not_snap_a_fixed_split() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let _grid = m.window_open(buf, WINMETHOD_ABOVE | WINMETHOD_FIXED, 1, 4, 0).unwrap();
        let leaves = m.relayout(81, 41, (9, 19));
        let right = leaves.iter().map(|(_, _, r, _)| r.left + r.width).max().unwrap();
        let bottom = leaves.iter().map(|(_, _, r, _)| r.top + r.height).max().unwrap();
        assert_eq!(right, 81, "no proportional split → full width used");
        assert_eq!(bottom, 41, "no proportional split → full height used");
    }

    // A grid split reports its parent pair's border hint (SQ-0286): the default
    // (Border, bit clear) → Some(true); winmethod_NoBorder → Some(false); a
    // parentless root → None.
    #[test]
    fn relayout_reports_border_presence_from_parent_split_method() {
        // Default (Border): the grid leaf reports Some(true).
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root
        let grid = m.window_open(buf, WINMETHOD_ABOVE | WINMETHOD_FIXED, 1, 4, 0).unwrap();
        let leaves = m.relayout(80, 24, (9, 19));
        let grid_border = leaves.iter().find(|(id, ..)| *id == grid).map(|&(.., b)| b);
        assert_eq!(grid_border, Some(Some(true)), "default split (Border) hints a framed grid leaf");

        // A lone, unsplit root has no parent pair → no border preference (None).
        let mut m = Model::new();
        let root = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root, never split
        let leaves = m.relayout(80, 24, (9, 19));
        let root_border = leaves.iter().find(|(id, ..)| *id == root).map(|&(.., b)| b);
        assert_eq!(root_border, Some(None), "a parentless root expresses no border preference");

        // NoBorder: the grid leaf reports Some(false).
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let grid = m
            .window_open(buf, WINMETHOD_ABOVE | WINMETHOD_FIXED | WINMETHOD_NOBORDER, 1, 4, 0)
            .unwrap();
        let leaves = m.relayout(80, 24, (9, 19));
        let grid_border = leaves.iter().find(|(id, ..)| *id == grid).map(|&(.., b)| b);
        assert_eq!(grid_border, Some(Some(false)), "NoBorder split hints an unframed grid leaf");
    }

    // A live mouse request must survive a Glk-chunk save/restore, exactly like a
    // char request.
    #[test]
    fn mouse_request_round_trips_through_serialize() {
        let mut m = Model::new();
        let grid = m.window_open(0, 0, 0, 4, 0).unwrap(); // TextGrid root
        m.set_mouse_request(grid);
        assert!(m.mouse_requested(grid), "armed before save");

        let restored = Model::deserialize(&m.serialize()).expect("round-trip");
        assert!(restored.mouse_requested(grid), "mouse request survived the round-trip");
        assert_eq!(
            restored.mouse_windows(),
            vec![(grid, WinType::TextGrid, Rect::default())],
            "restored model still enumerates the watching window",
        );
    }

    #[test]
    fn hyperlink_request_and_stream_link_round_trip_through_serialize() {
        let mut m = Model::new();
        let grid = m.window_open(0, 0, 0, 4, 0).unwrap(); // TextGrid root
        m.set_hyperlink_request(grid);
        // Set a current link value on the window's output stream.
        let sid = m.window_stream(grid).expect("window has a stream");
        m.set_stream_link(sid, 0xABCD);
        assert!(m.hyperlink_requested(grid), "armed before save");

        let restored = Model::deserialize(&m.serialize()).expect("round-trip");
        assert!(restored.hyperlink_requested(grid), "hyperlink request survived the round-trip");
        assert_eq!(
            restored.stream_kind_style(sid).map(|(_, _, link)| link),
            Some(0xABCD),
            "the stream's current link value survived the round-trip",
        );
    }

    // A pre-SQ-0308 (version 4) snapshot — no trailing resource_streams table,
    // no stream-kind tag 4 — must still deserialize, with the resource table
    // empty and all other state intact. Pinned wire bytes, hand-built to the v4
    // layout, so this exercises the real `version == 4` compat gates rather
    // than whatever `serialize()` currently emits.
    #[test]
    fn deserializes_pinned_version_4_snapshot_without_resource_table() {
        let mut b = Vec::new();
        let w = |b: &mut Vec<u8>, v: u32| b.extend_from_slice(&v.to_be_bytes());
        w(&mut b, 4); // version 4 (pre resource streams)
        w(&mut b, 0); // root window
        w(&mut b, 1); // current stream
        w(&mut b, 0); // window count
        // One stream slot: a memory stream mid-read.
        w(&mut b, 1); // stream count
        w(&mut b, 1); // present
        w(&mut b, 1); // id
        w(&mut b, 7); // rock
        w(&mut b, 0); // style Normal
        w(&mut b, 0); // link
        w(&mut b, 2); // read_count
        w(&mut b, 3); // write_count
        w(&mut b, 1); // kind = Memory
        w(&mut b, 0x1000); // addr
        w(&mut b, 64); // len
        w(&mut b, 5); // pos
        w(&mut b, 0); // unicode = false
        // VFS: one file "f" = [9, 9].
        w(&mut b, 1);
        w(&mut b, 1);
        b.push(b'f');
        w(&mut b, 2);
        b.extend_from_slice(&[9, 9]);
        w(&mut b, 0); // fileref count
        w(&mut b, 0); // file_streams count
        // v4 ends HERE — no resource_streams table follows.

        let m = Model::deserialize(&b).expect("a v4 snapshot still restores");
        assert_eq!(m.current_stream(), 1);
        assert_eq!(m.stream_rock(1), Some(7));
        assert_eq!(
            m.memory_stream_read_info(1),
            Some((0x1000, 64, 5, false)),
            "memory stream state intact"
        );
        assert_eq!(m.files.get("f").map(|v| v.as_slice()), Some(&[9u8, 9][..]), "VFS intact");
        assert!(m.resource_streams.is_empty(), "no resource streams in a v4 snapshot");
        assert_eq!(m.resource_stream_is_unicode(1), None, "stream 1 is Memory, not Resource");

        // And the restored model re-serializes as v5, round-tripping cleanly.
        let again = Model::deserialize(&m.serialize()).expect("v5 re-serialize round-trips");
        assert_eq!(again.memory_stream_read_info(1), Some((0x1000, 64, 5, false)));
    }

    // mouse_windows enumerates only windows with an armed request.
    #[test]
    fn mouse_windows_filters_to_armed_requests() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root
        let grid = m.window_open(buf, WINMETHOD_ABOVE | WINMETHOD_FIXED, 1, 4, 0).unwrap();
        m.relayout(80, 40, (9, 19));
        assert!(m.mouse_windows().is_empty(), "nothing armed → empty");
        m.set_mouse_request(grid);
        let armed = m.mouse_windows();
        assert_eq!(armed.len(), 1, "only the grid is armed");
        assert_eq!(armed[0].0, grid);
        assert_eq!(armed[0].1, WinType::TextGrid);
    }
}

#[cfg(test)]
mod style_hint_tests {
    use super::*;

    // stylehint_TextColor(7)/BackColor(8) are stored as 24-bit RGB per style,
    // and style_colour reads them back for a text-buffer window.
    #[test]
    fn text_and_back_colour_hints_stored_per_style() {
        let mut m = Model::new();
        // wintype_TextBuffer = 3, style_Header = 3.
        m.set_style_hint(3, 3, 7, 0x00FF_8040); // TextColor
        m.set_style_hint(3, 3, 8, 0x0011_2233); // BackColor
        let sc = m.style_colour(WinType::TextBuffer, GlkStyle::Header);
        assert_eq!(sc.fg, Some(0x00FF_8040));
        assert_eq!(sc.bg, Some(0x0011_2233));
        assert!(!sc.reverse);
        // A different style is untouched.
        assert_eq!(m.style_colour(WinType::TextBuffer, GlkStyle::Normal), StyleColour::default());
    }

    // The high byte of a colour value is masked off (Glk colours are 24-bit).
    #[test]
    fn colour_value_masked_to_24_bits() {
        let mut m = Model::new();
        m.set_style_hint(3, 0, 7, 0xFF12_3456);
        assert_eq!(m.style_colour(WinType::TextBuffer, GlkStyle::Normal).fg, Some(0x0012_3456));
    }

    // ReverseColor(9) sets/clears the reverse flag.
    #[test]
    fn reverse_colour_hint_toggles_flag() {
        let mut m = Model::new();
        m.set_style_hint(4, 5, 9, 1); // grid, style_Alert, reverse on
        assert!(m.style_colour(WinType::TextGrid, GlkStyle::Alert).reverse);
        m.set_style_hint(4, 5, 9, 0);
        assert!(!m.style_colour(WinType::TextGrid, GlkStyle::Alert).reverse);
    }

    // wintype_AllTypes (0) applies a hint to both the buffer and grid rows.
    #[test]
    fn all_types_applies_to_buffer_and_grid() {
        let mut m = Model::new();
        m.set_style_hint(0, 0, 7, 0x00AABBCC);
        assert_eq!(m.style_colour(WinType::TextBuffer, GlkStyle::Normal).fg, Some(0x00AABBCC));
        assert_eq!(m.style_colour(WinType::TextGrid, GlkStyle::Normal).fg, Some(0x00AABBCC));
    }

    // Buffer and grid hints for the same style are independent.
    #[test]
    fn buffer_and_grid_rows_are_independent() {
        let mut m = Model::new();
        m.set_style_hint(3, 1, 7, 0x00111111); // buffer
        m.set_style_hint(4, 1, 7, 0x00222222); // grid
        assert_eq!(m.style_colour(WinType::TextBuffer, GlkStyle::Emphasized).fg, Some(0x00111111));
        assert_eq!(m.style_colour(WinType::TextGrid, GlkStyle::Emphasized).fg, Some(0x00222222));
    }

    // Out-of-range style numbers and unsupported wintypes are ignored (no panic).
    #[test]
    fn out_of_range_inputs_ignored() {
        let mut m = Model::new();
        m.set_style_hint(3, NUMSTYLES, 7, 0x00FFFFFF); // style too large
        m.set_style_hint(99, 0, 7, 0x00FFFFFF);        // bad wintype
        assert_eq!(m.style_colour(WinType::TextBuffer, GlkStyle::Normal), StyleColour::default());
    }

    // clear_style_hint removes a previously set colour hint.
    #[test]
    fn clear_resets_hint() {
        let mut m = Model::new();
        m.set_style_hint(3, 0, 7, 0x00FFFFFF);
        m.set_style_hint(3, 0, 8, 0x00000000);
        m.clear_style_hint(3, 0, 7);
        let sc = m.style_colour(WinType::TextBuffer, GlkStyle::Normal);
        assert_eq!(sc.fg, None, "text colour cleared");
        assert_eq!(sc.bg, Some(0), "back colour retained");
    }

    // A pair window never carries colour.
    #[test]
    fn pair_window_has_no_colour() {
        let mut m = Model::new();
        m.set_style_hint(0, 0, 7, 0x00FFFFFF);
        assert_eq!(m.style_colour(WinType::Pair, GlkStyle::Normal), StyleColour::default());
    }

    // garglk_set_zcolors overrides the style-hint colour for text on that stream,
    // and an unset channel falls through to the hint.
    #[test]
    fn garglk_zcolors_override_layers_over_style_hint() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root
        let sid = m.window_stream(buf).expect("window has a stream");
        m.set_style_hint(3, 0, 7, 0x00111111); // Normal fg hint
        m.set_style_hint(3, 0, 8, 0x00222222); // Normal bg hint
        // Override fg only; bg falls through to the hint.
        m.set_stream_zcolors(sid, 0x00AABBCC, Model::ZCOLOR_CURRENT);
        let sc = m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal);
        assert_eq!(sc.fg, Some(0x00AABBCC), "fg overridden");
        assert_eq!(sc.bg, Some(0x00222222), "bg keeps the style hint (Current)");
    }

    // High bits of a garglk RGB value are masked to 24 bits (as with style hints).
    #[test]
    fn garglk_zcolors_masks_to_24_bits() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let sid = m.window_stream(buf).unwrap();
        m.set_stream_zcolors(sid, 0x00FF_FFFB, Model::ZCOLOR_CURRENT); // 0x00FFFFFB: not a sentinel
        assert_eq!(m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal).fg, Some(0x00FF_FFFB));
    }

    // zcolor_Default clears a previously set override, falling back to the hint.
    #[test]
    fn garglk_zcolors_default_clears_override() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let sid = m.window_stream(buf).unwrap();
        m.set_style_hint(3, 0, 7, 0x00111111);
        m.set_stream_zcolors(sid, 0x00AABBCC, Model::ZCOLOR_CURRENT);
        m.set_stream_zcolors(sid, Model::ZCOLOR_DEFAULT, Model::ZCOLOR_CURRENT); // reset fg
        assert_eq!(
            m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal).fg,
            Some(0x00111111),
            "fg back to the style hint after Default",
        );
    }

    // Transparent and Cursor sentinels leave the channel unchanged.
    #[test]
    fn garglk_zcolors_transparent_and_cursor_are_noops() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let sid = m.window_stream(buf).unwrap();
        m.set_stream_zcolors(sid, 0x00AABBCC, 0x00445566);
        m.set_stream_zcolors(sid, Model::ZCOLOR_TRANSPARENT, Model::ZCOLOR_CURSOR);
        let sc = m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal);
        assert_eq!(sc.fg, Some(0x00AABBCC), "Transparent leaves fg unchanged");
        assert_eq!(sc.bg, Some(0x00445566), "Cursor leaves bg unchanged");
    }

    // garglk_set_reversevideo forces the reverse flag on/off, overriding the hint.
    #[test]
    fn garglk_reversevideo_forces_reverse() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let sid = m.window_stream(buf).unwrap();
        m.set_stream_reversevideo(sid, 1);
        assert!(m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal).reverse);
        m.set_stream_reversevideo(sid, 0);
        assert!(!m.stream_style_colour(sid, WinType::TextBuffer, GlkStyle::Normal).reverse);
    }

    // The override is per-stream: colours set on one stream don't affect another.
    #[test]
    fn garglk_override_is_per_stream() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap();
        let sid = m.window_stream(buf).unwrap();
        m.set_stream_zcolors(sid, 0x00AABBCC, Model::ZCOLOR_CURRENT);
        // A fresh memory stream carries no override.
        let other = m.stream_open_memory(0, 0, false, 0);
        assert_eq!(m.stream_style_colour(other, WinType::TextBuffer, GlkStyle::Normal), StyleColour::default());
    }

    #[test]
    fn fileref_iterate_rock_and_destroy() {
        let mut m = Model::new();
        let a = m.fileref_create(0x00, "save".to_string(), 0x11);
        let b = m.fileref_create(0x00, "auto".to_string(), 0x22);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
        // Rocks round-trip.
        assert_eq!(m.fileref_rock(a), 0x11);
        assert_eq!(m.fileref_rock(b), 0x22);
        // Iterate walks both live filerefs then returns (0, 0).
        let (first, first_rock) = m.fileref_iterate(0);
        assert_eq!(first, a);
        assert_eq!(first_rock, 0x11);
        let (second, second_rock) = m.fileref_iterate(first);
        assert_eq!(second, b);
        assert_eq!(second_rock, 0x22);
        assert_eq!(m.fileref_iterate(second), (0, 0));
        // After destroying the first, it drops out of iteration.
        m.fileref_destroy(a);
        assert_eq!(m.fileref_iterate(0), (b, 0x22));
    }

    #[test]
    fn fileref_exists_and_delete_track_the_vfs() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "data".to_string(), 0);
        // A never-written fileref does not exist.
        assert!(!m.fileref_exists(f));
        // Once its file has bytes it exists; delete removes it.
        m.files.insert("data".to_string(), vec![1, 2, 3]);
        assert!(m.fileref_exists(f));
        m.fileref_delete(f);
        assert!(!m.fileref_exists(f));
    }

    #[test]
    fn savedgame_save_marks_existence_and_end_position_reports_size() {
        // Reproduces the Inform/Glk library's post-`@save` verification (as used
        // by CounterfeitMonkey's SAVE verb): open a SavedGame slot for writing,
        // record the save's byte count, then reopen it for reading and seek to
        // the end — the reported position must equal the save size (non-zero) and
        // the slot must exist. Before the fix a SavedGame `Null` stream reported
        // position 0 and the slot never "existed" (host `.qzl` saves are
        // decoupled from the VFS), so the game read the save back as empty and
        // printed "Save failed." even though the host wrote the file.
        let mut m = Model::new();
        // usage 0x01 = fileusage_SavedGame.
        let fref = m.fileref_create(0x01, "slot".to_string(), 0);
        assert!(!m.fileref_exists(fref), "a never-saved slot does not exist");

        // The game opens the slot for writing; `@save`'s shim credits the byte
        // count via note_stream_write (save_quetzal().len()).
        let wstr = m.stream_open_file(fref, 0x01, false, 0);
        assert_ne!(wstr, 0);
        m.note_stream_write(wstr, 49336);
        assert_eq!(m.stream_close(wstr), Some((0, 49336)));

        // Verification step 1: the slot now exists.
        assert!(m.fileref_exists(fref), "a written SavedGame slot exists");
        // Verification step 2: reopen for read, seek to end, read the position —
        // it reports the save's byte count, not 0.
        let rstr = m.stream_open_file(fref, 0x02, false, 0);
        assert_ne!(rstr, 0);
        m.stream_set_position(rstr, 0, 2); // seekmode 2 = from end
        assert_eq!(
            m.stream_position(rstr),
            Some(49336),
            "seek-to-end on a saved slot reports its byte count, not 0",
        );
        m.stream_close(rstr);

        // Deleting the slot clears both existence and the recorded size.
        m.fileref_delete(fref);
        assert!(!m.fileref_exists(fref), "delete removes the slot");
    }

    #[test]
    fn seeded_saved_game_slot_is_visible_without_an_in_session_save() {
        // A host reseeds saved_game_files from disk at boot (SQ-0301). A slot
        // seeded that way must read as present at its recorded size to a
        // create_by_name game that only probes/restores — no in-session @save.
        let mut m = Model::new();
        m.seed_saved_game_file("slot".to_string(), 1234);

        // The game's create_by_name fileref on that slot now exists.
        let fref = m.fileref_create(0x01, "slot".to_string(), 0);
        assert!(m.fileref_exists(fref), "a seeded slot exists across launches");

        // Reopen for read, seek to end: the position reports the seeded size,
        // so a game gating @restore on it sees its save.
        let rstr = m.stream_open_file(fref, 0x02, false, 0);
        assert_ne!(rstr, 0);
        m.stream_set_position(rstr, 0, 2); // seekmode 2 = from end
        assert_eq!(m.stream_position(rstr), Some(1234), "seek-to-end reports the seeded size");
    }

    #[test]
    fn encode_files_roundtrips_and_skips_temp() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("save".to_string(), vec![1, 2, 3]);
        m.insert("data".to_string(), b"hello".to_vec());
        m.insert("__temp_0__".to_string(), vec![9, 9, 9]);
        let decoded = decode_files(&encode_files(&m));
        // The __temp_ key is dropped; the rest round-trips exactly.
        let mut expected = m.clone();
        expected.remove("__temp_0__");
        assert_eq!(decoded, expected);

        // Tolerant decode: garbage, too-short, and wrong-magic bytes → empty map.
        assert!(decode_files(&[]).is_empty());
        assert!(decode_files(&[0xDE, 0xAD]).is_empty());
        assert!(decode_files(b"XXXX\x00\x00\x00\x01\x00\x00\x00\x00").is_empty());
        // Right magic + version but a count that overruns the buffer → empty.
        assert!(decode_files(b"GVFS\x00\x00\x00\x01\x00\x00\x00\x09").is_empty());
        // An empty (count 0) but otherwise valid blob decodes to an empty map.
        assert!(decode_files(&encode_files(&std::collections::BTreeMap::new())).is_empty());
    }

    #[test]
    fn vfs_dirty_tracks_mutations() {
        let mut m = Model::new();
        assert!(!m.vfs_dirty(), "a fresh model is not dirty");

        let f = m.fileref_create(0x00, "f".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert!(m.vfs_dirty(), "opening for Write truncates/creates → dirty");

        m.clear_vfs_dirty();
        assert!(!m.vfs_dirty(), "clear resets the flag");

        m.file_stream_write(sid, "Hi");
        assert!(m.vfs_dirty(), "writing bytes re-dirties");

        m.clear_vfs_dirty();
        m.fileref_delete(f);
        assert!(m.vfs_dirty(), "deleting a file dirties");

        // A read-only open does not dirty.
        m.files.insert("r".to_string(), vec![1]);
        let rf = m.fileref_create(0x00, "r".to_string(), 0);
        m.clear_vfs_dirty();
        m.stream_open_file(rf, FM_READ, false, 0);
        assert!(!m.vfs_dirty(), "a Read-mode open is not a mutation");
    }

    #[test]
    fn fileref_sanitizes_names() {
        assert_eq!(Model::sanitize_fileref_name("a/b*c.sav"), "a_b_c.sav");
        assert_eq!(Model::sanitize_fileref_name(""), "file");
        assert_eq!(Model::sanitize_fileref_name("///"), "___");
        assert_eq!(Model::sanitize_fileref_name("Ok-Name_1.dat"), "Ok-Name_1.dat");
    }

    #[test]
    fn file_names_lists_user_files_hiding_internal_keys() {
        let mut m = Model::new();
        m.files.insert("story-notes".to_string(), vec![1, 2, 3]);
        m.files.insert("__temp_0__".to_string(), vec![4]);
        m.files.insert("__prompt_2__".to_string(), vec![5]);
        let names = m.file_names();
        assert_eq!(names, vec!["story-notes".to_string()], "temp/legacy-prompt keys are hidden");
    }

    // Glk filemode constants (garglk glk.h).
    const FM_WRITE: u32 = 0x01;
    const FM_READ: u32 = 0x02;
    const FM_READWRITE: u32 = 0x03;
    const FM_WRITEAPPEND: u32 = 0x05;

    #[test]
    fn file_stream_write_truncates_existing() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        m.files.insert("f".to_string(), vec![b'O', b'L', b'D']);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert_ne!(sid, 0);
        assert_eq!(m.files["f"], Vec::<u8>::new(), "Write truncates on open");
        m.file_stream_write(sid, "Hi");
        assert_eq!(m.files["f"], b"Hi");
    }

    #[test]
    fn file_stream_write_append_preserves_and_seeks_end() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        m.files.insert("f".to_string(), b"AB".to_vec());
        let sid = m.stream_open_file(f, FM_WRITEAPPEND, false, 0);
        assert_eq!(m.stream_position(sid), Some(2), "append seeks to end");
        m.file_stream_write(sid, "CD");
        assert_eq!(m.files["f"], b"ABCD");
    }

    #[test]
    fn note_stream_write_bumps_write_count_without_storing_bytes() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert_ne!(sid, 0);

        m.note_stream_write(sid, 42);

        assert_eq!(m.stream_close(sid), Some((0, 42)), "write_count credited, no reads");
        assert_eq!(m.files["f"], Vec::<u8>::new(), "no bytes were actually stored");
    }

    #[test]
    fn savedgame_stream_is_a_null_conduit_decoupled_from_vfs() {
        let mut m = Model::new();
        // usage 0x01 = fileusage_SavedGame.
        let f = m.fileref_create(0x01, "s".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert_ne!(sid, 0, "SavedGame Write opens a conduit");
        assert!(!m.files.contains_key("s"), "no VFS entry is created for a SavedGame stream");

        // Read succeeds even though nothing was ever saved this session.
        let f2 = m.fileref_create(0x01, "s".to_string(), 0);
        let rsid = m.stream_open_file(f2, FM_READ, false, 0);
        assert_ne!(rsid, 0, "Read on a SavedGame conduit succeeds with no prior save");

        // Writes discard; the VFS stays empty.
        m.file_stream_write(sid, "hello");
        assert!(!m.files.contains_key("s"), "writes to a null conduit are discarded");

        // Reads return EOF.
        assert_eq!(m.file_stream_read_char(rsid), None, "a null conduit reads as EOF");
    }

    #[test]
    fn savedgame_conduit_never_persists_into_the_vfs() {
        let mut m = Model::new();
        let f = m.fileref_create(0x01, "save".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        // The `@save` write-count shim (Task 3) still works on a null conduit.
        m.note_stream_write(sid, 1234);
        m.file_stream_write(sid, "ignored");
        assert!(m.files.is_empty(), "a SavedGame conduit never materializes a VFS entry");
    }

    #[test]
    fn non_savedgame_usages_still_use_the_real_vfs_unchanged() {
        let mut m = Model::new();
        // usage 0x00 = fileusage_Data: Write creates a VFS entry, Read fails absent.
        let f = m.fileref_create(0x00, "notes".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert_ne!(sid, 0);
        assert!(m.files.contains_key("notes"), "a Data Write still creates a VFS entry");

        let f2 = m.fileref_create(0x00, "missing".to_string(), 0);
        assert_eq!(m.stream_open_file(f2, FM_READ, false, 0), 0, "Data Read still fails if absent");
    }

    #[test]
    fn file_stream_read_missing_returns_zero_and_allocates_nothing() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        let sid = m.stream_open_file(f, FM_READ, false, 0);
        assert_eq!(sid, 0, "Read of a missing file fails");
        assert_eq!(m.stream_iterate(0), (0, 0), "no stream was allocated");
    }

    #[test]
    fn file_stream_seek_modes_clamp() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        m.files.insert("f".to_string(), b"ABCDE".to_vec()); // len 5
        let sid = m.stream_open_file(f, FM_READWRITE, false, 0);
        m.stream_set_position(sid, 2, 0); // Start + 2
        assert_eq!(m.stream_position(sid), Some(2));
        m.stream_set_position(sid, 1, 1); // Current + 1
        assert_eq!(m.stream_position(sid), Some(3));
        m.stream_set_position(sid, 0, 2); // End
        assert_eq!(m.stream_position(sid), Some(5));
        m.stream_set_position(sid, -10, 1); // clamp low
        assert_eq!(m.stream_position(sid), Some(0));
        m.stream_set_position(sid, 100, 0); // clamp high
        assert_eq!(m.stream_position(sid), Some(5));
    }

    #[test]
    fn file_stream_uni_binary_roundtrips_big_endian() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "u".to_string(), 0); // binary usage (no TextMode)
        let sid = m.stream_open_file(f, FM_WRITE, true, 0);
        m.file_stream_write(sid, "\u{1F600}"); // one astral code point
        assert_eq!(m.files["u"], vec![0x00, 0x01, 0xF6, 0x00], "4-byte big-endian");
        m.stream_close(sid);
        let sid2 = m.stream_open_file(f, FM_READ, true, 0);
        assert_eq!(m.file_stream_read_char(sid2), Some(0x1F600));
        assert_eq!(m.file_stream_read_char(sid2), None, "EOF");
    }

    #[test]
    fn file_stream_close_syncs_counts_and_frees_slot() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "f".to_string(), 0);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        m.file_stream_write(sid, "Hi");
        assert_eq!(m.stream_close(sid), Some((0, 2)), "read_count 0, write_count 2");
        assert_eq!(m.files["f"], b"Hi", "bytes persist after close");
        assert_eq!(m.stream_position(sid), None, "stream slot is freed");
        assert!(m.file_streams.is_empty(), "cursor side-table entry dropped");
    }

    // A write whose cursor sits past the current end (only reachable when a
    // second stream truncated the file) zero-fills the gap instead of misplacing
    // the byte — the write lands at its true offset.
    #[test]
    fn vfs_write_byte_zero_fills_a_gap_past_end() {
        let mut data = Vec::new();
        let mut pos = 3usize;
        Model::vfs_write_byte(&mut data, &mut pos, b'X');
        assert_eq!(data, vec![0, 0, 0, b'X']);
        assert_eq!(pos, 4);
    }

    // The whole VFS — file bytes, the fileref table, and an OPEN file stream's
    // cursor — must survive a Glk snapshot round-trip (version 4).
    #[test]
    fn vfs_and_file_stream_survive_snapshot_round_trip() {
        let mut m = Model::new();
        let f = m.fileref_create(0x00, "save".to_string(), 0x42);
        let sid = m.stream_open_file(f, FM_WRITE, false, 0);
        assert_ne!(sid, 0);
        m.file_stream_write(sid, "HELLO");
        // Seek to a known position and leave the stream OPEN.
        m.stream_set_position(sid, 1, 0); // Start + 1
        assert_eq!(m.stream_position(sid), Some(1));

        let mut restored = Model::deserialize(&m.serialize()).expect("round-trip");
        // File bytes are intact in the restored VFS.
        assert_eq!(restored.files["save"], b"HELLO");
        // The fileref still exists and iteration yields it (id + rock preserved).
        assert!(restored.fileref_exists(f));
        assert_eq!(restored.fileref_iterate(0), (f, 0x42));
        // The still-open file stream reports the same cursor ...
        assert_eq!(restored.stream_position(sid), Some(1));
        // ... and reads the expected next byte ('E' at pos 1 of "HELLO").
        assert_eq!(restored.file_stream_read_char(sid), Some(b'E' as u32));
    }

    // A file stream must coexist with the other stream kinds across a snapshot:
    // a Window stream (tag 0) and a Memory stream (tag 1) still round-trip
    // alongside the new File stream (tag 2) and the VFS payload.
    #[test]
    fn file_memory_and_window_streams_coexist_through_snapshot() {
        let mut m = Model::new();
        let buf = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer root (+ window stream)
        let mem = m.stream_open_memory(0x1000, 64, false, 0x7);
        let f = m.fileref_create(0x00, "d".to_string(), 0);
        let fsid = m.stream_open_file(f, FM_WRITE, false, 0);
        m.file_stream_write(fsid, "Z");

        let restored = Model::deserialize(&m.serialize()).expect("round-trip");
        // The window's output stream survived.
        assert!(restored.window_stream(buf).is_some());
        // The memory stream survived with its addr/len.
        assert!(matches!(
            restored.stream_kind_style(mem).map(|(k, _, _)| k),
            Some(StreamKind::Memory { addr: 0x1000, len: 64, .. }),
        ));
        // The file stream + its bytes survived.
        assert_eq!(restored.files["d"], b"Z");
        assert_eq!(restored.stream_position(fsid), Some(1));
    }

    // ── date/time (Glk 0.7.6) ─────────────────────────────────────────────────

    use super::datetime as dt;

    /// The epoch second count for a UTC date, via the round-trip conversion.
    fn epoch(y: i32, mo: i32, d: i32, h: i32, mi: i32, s: i32) -> i64 {
        let (secs, _) = dt::date_to_time(dt::GlkDate {
            year: y, month: mo, day: d, weekday: 0, hour: h, minute: mi, second: s, microsec: 0,
        });
        secs
    }

    #[test]
    fn epoch_zero_is_1970_new_year_thursday() {
        let d = dt::time_to_date(0, 0);
        assert_eq!((d.year, d.month, d.day), (1970, 1, 1));
        assert_eq!((d.hour, d.minute, d.second), (0, 0, 0));
        assert_eq!(d.weekday, 4, "1970-01-01 is a Thursday (Sunday=0)");
        assert_eq!(epoch(1970, 1, 1, 0, 0, 0), 0);
    }

    #[test]
    fn known_timestamp_round_trips_both_ways() {
        // 2026-07-14 12:00:00 UTC = 1_784_030_400 (verified against date -u).
        let secs = 1_784_030_400;
        let d = dt::time_to_date(secs, 123_456);
        assert_eq!((d.year, d.month, d.day), (2026, 7, 14));
        assert_eq!((d.hour, d.minute, d.second), (12, 0, 0));
        assert_eq!(d.weekday, 2, "2026-07-14 is a Tuesday");
        assert_eq!(d.microsec, 123_456, "microsec carried through unchanged");
        assert_eq!(epoch(2026, 7, 14, 12, 0, 0), secs, "date -> time inverts time -> date");
    }

    #[test]
    fn negative_pre_1970_time_floors_correctly() {
        // 1969-12-31 23:59:59 UTC = -1.
        let d = dt::time_to_date(-1, 0);
        assert_eq!((d.year, d.month, d.day), (1969, 12, 31));
        assert_eq!((d.hour, d.minute, d.second), (23, 59, 59));
        assert_eq!(d.weekday, 3, "1969-12-31 is a Wednesday");
        assert_eq!(epoch(1969, 12, 31, 23, 59, 59), -1);
        // A full day before the epoch.
        assert_eq!(epoch(1969, 12, 31, 0, 0, 0), -86400);
    }

    #[test]
    fn leap_day_2000_02_29_is_valid() {
        let secs = epoch(2000, 2, 29, 0, 0, 0);
        let d = dt::time_to_date(secs, 0);
        assert_eq!((d.year, d.month, d.day), (2000, 2, 29));
        assert_eq!(d.weekday, 2, "2000-02-29 is a Tuesday");
    }

    #[test]
    fn out_of_range_date_fields_normalize_like_timegm() {
        // Jan 32 -> Feb 1; month 13 -> next January; 25:00 -> next day 01:00.
        assert_eq!(
            dt::time_to_date(epoch(2001, 1, 32, 0, 0, 0), 0),
            dt::time_to_date(epoch(2001, 2, 1, 0, 0, 0), 0),
        );
        let d = dt::time_to_date(epoch(2001, 13, 1, 0, 0, 0), 0);
        assert_eq!((d.year, d.month, d.day), (2002, 1, 1));
        let d = dt::time_to_date(epoch(2001, 6, 1, 25, 0, 0), 0);
        assert_eq!((d.day, d.hour), (2, 1));
    }

    #[test]
    fn microsecond_carry_matches_cgdate() {
        // >= 1e6 carries whole seconds up; the remainder stays in microsec.
        let (secs, micro) = dt::date_to_time(dt::GlkDate {
            year: 1970, month: 1, day: 1, weekday: 0, hour: 0, minute: 0, second: 0,
            microsec: 2_500_000,
        });
        assert_eq!((secs, micro), (2, 500_000));
        // Negative microsec borrows a second and normalizes into 0..=999999.
        let (secs, micro) = dt::date_to_time(dt::GlkDate {
            year: 1970, month: 1, day: 1, weekday: 0, hour: 0, minute: 0, second: 5,
            microsec: -1,
        });
        assert_eq!((secs, micro), (4, 999_999));
    }

    #[test]
    fn simple_time_floors_toward_negative_infinity() {
        assert_eq!(dt::simplify_time(100, 60), 1);
        assert_eq!(dt::simplify_time(59, 60), 0);
        // -1 second / 60 rounds DOWN to -1, not toward zero.
        assert_eq!(dt::simplify_time(-1, 60), -1);
        assert_eq!(dt::simplify_time(-60, 60), -1);
        assert_eq!(dt::simplify_time(-61, 60), -2);
        assert_eq!(dt::simplify_time(5, 0), 0, "factor 0 is invalid -> 0");
    }

    #[test]
    fn simple_time_to_date_multiplies_by_factor() {
        // A simple time of 100 minutes (factor 60) = 6000s = 1970-01-01 01:40:00.
        let d = dt::time_to_date(100i64 * 60, 0);
        assert_eq!((d.hour, d.minute), (1, 40));
    }

    // ── echo streams (Glk 0.7.6 §3.6) ─────────────────────────────────────────

    #[test]
    fn echo_stream_set_and_get_round_trip() {
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap();
        let mem = m.stream_open_memory(0x1000, 64, false, 0);
        assert_eq!(m.window_echo_stream(win), 0, "no echo by default");
        m.window_set_echo_stream(win, mem);
        assert_eq!(m.window_echo_stream(win), mem, "get returns the set echo stream");
        m.window_set_echo_stream(win, 0);
        assert_eq!(m.window_echo_stream(win), 0, "cleared with str = 0");
    }

    #[test]
    fn echo_stream_refuses_self_loop() {
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap();
        let own = m.window_stream(win).unwrap();
        m.window_set_echo_stream(win, own); // illegal per §3.6
        assert_eq!(m.window_echo_stream(win), 0, "self-loop refused, echo stays cleared");
    }

    #[test]
    fn closing_echo_stream_stops_the_window_echoing() {
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap();
        let mem = m.stream_open_memory(0x1000, 64, false, 0);
        m.window_set_echo_stream(win, mem);
        m.stream_close(mem);
        assert_eq!(m.window_echo_stream(win), 0, "closed echo stream is unhooked");
    }

    // ── style measurement (Glk 0.7.6 §5.5.2) ──────────────────────────────────

    #[test]
    fn style_measure_returns_set_colour_hints_and_unknown_otherwise() {
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap(); // TextBuffer
        // stylehint_set(wintype_TextBuffer=3, style_Note=6, TextColor=7, 0x00FF00).
        m.set_style_hint(3, 6, 7, 0x00FF00);
        m.set_style_hint(3, 6, 9, 1); // ReverseColor on
        // TextColor is known and returns the set value.
        assert_eq!(m.style_measure(win, 6, 7), Some(0x00FF00), "TextColor measured");
        // ReverseColor is known (effective flag).
        assert_eq!(m.style_measure(win, 6, 9), Some(1), "ReverseColor measured");
        // BackColor was never set for this style -> unknown.
        assert_eq!(m.style_measure(win, 6, 8), None, "unset BackColor is unknown");
        // Weight/Size/Justification are not tracked -> unknown.
        assert_eq!(m.style_measure(win, 6, 4), None, "Weight untracked -> unknown");
        assert_eq!(m.style_measure(win, 6, 3), None, "Size untracked -> unknown");
        // A style with no hints reports its default reverse (false) but unknown colour.
        assert_eq!(m.style_measure(win, 0, 7), None, "Normal has no TextColor hint");
        assert_eq!(m.style_measure(win, 0, 9), Some(0), "Normal reverse defaults off");
    }

    #[test]
    fn style_distinguish_flips_on_a_differing_recorded_hint() {
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap();
        // Same style number is never distinguishable.
        assert!(!m.style_distinguish(win, 3, 3), "identical style is indistinguishable");
        // Two styles with no recorded hints look the same to the model.
        assert!(!m.style_distinguish(win, 0, 3), "unhinted styles are indistinguishable");
        // Give Header a text colour: now it differs from Normal.
        m.set_style_hint(3, 3, 7, 0xFF0000); // style_Header=3 TextColor red
        assert!(m.style_distinguish(win, 0, 3), "a differing colour hint distinguishes them");
        // Reverse-only difference also distinguishes.
        m.set_style_hint(3, 4, 9, 1); // style_Subheader=4 ReverseColor on
        assert!(m.style_distinguish(win, 0, 4), "a reverse-video difference distinguishes");
    }

    #[test]
    fn style_distinguish_compares_rendered_weight_and_oblique_not_ignored_hints() {
        // SQ-0317: Weight (4) and Oblique (5) are rendered attributes a terminal
        // can show (bold / italic), so a style differing only in one of them is
        // distinguishable; Size/Indentation/Justification/Proportional are hints a
        // fixed-cell terminal cannot render, so they never distinguish.
        let mut m = Model::new();
        let win = m.window_open(0, 0, 0, 3, 0).unwrap(); // wintype_TextBuffer=3
        // A Weight hint on User1 (style 9) makes it differ from the unhinted User2 (10).
        m.set_style_hint(3, 9, 4, 1); // stylehint_Weight = bold
        assert!(m.style_distinguish(win, 9, 10), "a differing weight hint distinguishes");
        // Clearing it makes them indistinguishable again.
        m.clear_style_hint(3, 9, 4);
        assert!(!m.style_distinguish(win, 9, 10), "clearing the weight hint un-distinguishes");
        // An Oblique difference also distinguishes.
        m.set_style_hint(3, 9, 5, 1); // stylehint_Oblique = italic
        assert!(m.style_distinguish(win, 9, 10), "a differing oblique hint distinguishes");
        m.clear_style_hint(3, 9, 5);
        // An unrendered hint (Size = 3) set differently does NOT distinguish.
        m.set_style_hint(3, 9, 3, 20); // stylehint_Size — ignored
        assert!(!m.style_distinguish(win, 9, 10), "an unrendered size hint never distinguishes");
        // Identical styles with the same hints are never distinguishable.
        m.set_style_hint(3, 10, 4, 1);
        m.set_style_hint(3, 9, 4, 1);
        assert!(!m.style_distinguish(win, 9, 10), "matching weight hints are indistinguishable");
    }
}
