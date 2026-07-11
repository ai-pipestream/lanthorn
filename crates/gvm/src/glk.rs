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
    /// The resolved layout changed: each entry is `(window id, type, rect)` for
    /// every non-pair window, in window-id order.
    fn window_layout(&mut self, _wins: &[(u32, WinType, Rect)]) {}
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
    fn window_layout(&mut self, wins: &[(u32, WinType, Rect)]) {
        for &(id, _ty, rect) in wins {
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
}

/// The mutable read/write state of an open file stream, kept in a side table
/// (`Model::file_streams`) keyed by stream id so `StreamKind` stays `Copy`.
/// Declared here for Task 1; populated by the file-stream ops in Task 2.
#[allow(dead_code)] // fields are read by the file-stream ops added in Task 2
#[derive(Clone, Debug)]
struct FileStream {
    name: String,
    mode: u32,
    pos: usize,
    unicode: bool,
    usage: u32,
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
    /// Open file streams keyed by stream id (Task 2 populates this).
    #[allow(dead_code)] // read by the file-stream ops added in Task 2
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
    /// Pixel size of one character cell (w, h), set by `relayout` for graphics
    /// fixed-split conversion. (1,1) until the backend reports otherwise.
    char_px: (u32, u32),
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
            char_px: (1, 1),
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

    /// Record a `glk_stylehint_set(wintype, styl, hint, val)`. Only the colour
    /// hints are kept: TextColor (7), BackColor (8), ReverseColor (9); other
    /// hints and out-of-range styles are ignored.
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
        }
    }

    /// Undo a `glk_stylehint_clear(wintype, styl, hint)` for a colour hint.
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
    fn alloc_fileref(&mut self, usage: u32, name: String, rock: u32) -> u32 {
        let id = (self.filerefs.len() + 1) as u32;
        self.filerefs.push(Some(FileRef { id, rock, name, usage }));
        id
    }

    /// `glk_fileref_create_by_name`: sanitize `name`, allocate a fileref, return id.
    pub fn fileref_create(&mut self, usage: u32, name: String, rock: u32) -> u32 {
        let name = Self::sanitize_fileref_name(&name);
        self.alloc_fileref(usage, name, rock)
    }

    /// `glk_fileref_create_temp`: synthesize a unique name and allocate a fileref.
    pub fn fileref_create_temp(&mut self, usage: u32, rock: u32) -> u32 {
        let name = format!("__temp_{}__", self.filerefs.len());
        self.alloc_fileref(usage, name, rock)
    }

    /// `glk_fileref_create_by_prompt`: no TUI picker is available, so degrade to a
    /// fixed per-usage default name. (Known limitation.)
    pub fn fileref_create_by_prompt(&mut self, usage: u32, _fmode: u32, rock: u32) -> u32 {
        let name = format!("__prompt_{}__", usage & 0x0f);
        self.alloc_fileref(usage, name, rock)
    }

    /// `glk_fileref_create_from_fileref`: clone `oldfref`'s name with a new
    /// usage/rock. Returns 0 if `oldfref` is invalid.
    pub fn fileref_create_from(&mut self, usage: u32, oldfref: u32, rock: u32) -> u32 {
        match self.fileref(oldfref).map(|f| f.name.clone()) {
            Some(name) => self.alloc_fileref(usage, name, rock),
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
    /// written to the VFS.
    pub fn fileref_exists(&self, fref: u32) -> bool {
        match self.fileref(fref) {
            Some(f) => self.files.contains_key(&f.name),
            None => false,
        }
    }

    /// `glk_fileref_delete_file`: remove the fileref's file from the VFS.
    pub fn fileref_delete(&mut self, fref: u32) {
        if let Some(name) = self.fileref(fref).map(|f| f.name.clone()) {
            self.files.remove(&name);
        }
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
    /// returning the leaf-window layout `(id, type, rect)` in id order (to hand
    /// to [`GlkBackend::window_layout`]).
    pub fn relayout(&mut self, width: u32, height: u32, char_px: (u32, u32)) -> Vec<(u32, WinType, Rect)> {
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
                out.push((w.id, w.wintype, w.rect));
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
    /// Close a stream, returning its `(read_count, write_count)`. The current
    /// stream is cleared if it was this one.
    pub fn stream_close(&mut self, id: u32) -> Option<(u32, u32)> {
        let counts = self.stream(id).map(|s| (s.read_count, s.write_count))?;
        // Window streams are owned by their window; only free memory streams here.
        if let Some(s) = self.stream(id) {
            if matches!(s.kind, StreamKind::Memory { .. }) {
                self.streams[(id - 1) as usize] = None;
            }
        }
        if self.cur_stream == id {
            self.cur_stream = 0;
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
            StreamKind::Window(_) => Some(self.stream(id)?.write_count),
        }
    }
    /// Seek a memory stream. `seekmode`: 0 = from start, 1 = from current,
    /// 2 = from end; clamped to `[0, len]`.
    pub fn stream_set_position(&mut self, id: u32, pos: i32, seekmode: u32) {
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
                    }
                }
            }
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
        if version != GLK_SNAPSHOT_VERSION {
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
                hyperlink_req, terminators: Vec::new(), child1, child2, key, method, size,
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

        if !r.done() {
            return Err("Glk snapshot: trailing bytes".to_string());
        }
        Ok(Model {
            windows,
            streams,
            files: std::collections::BTreeMap::new(),
            filerefs: Vec::new(),
            file_streams: std::collections::BTreeMap::new(),
            root,
            cur_stream,
            events: std::collections::VecDeque::new(),
            style_hints: [[StyleColour::default(); NUMSTYLES as usize]; 2],
            char_px: (1, 1),
        })
    }
}

/// Version tag at the head of a `Glk ` snapshot chunk (bumped on a format change).
const GLK_SNAPSHOT_VERSION: u32 = 3;

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
        let right = leaves.iter().map(|(_, _, r)| r.left + r.width).max().unwrap();
        let bottom = leaves.iter().map(|(_, _, r)| r.top + r.height).max().unwrap();
        assert_eq!(right, 81, "no proportional split → full width used");
        assert_eq!(bottom, 41, "no proportional split → full height used");
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
    fn fileref_sanitizes_names() {
        assert_eq!(Model::sanitize_fileref_name("a/b*c.sav"), "a_b_c.sav");
        assert_eq!(Model::sanitize_fileref_name(""), "file");
        assert_eq!(Model::sanitize_fileref_name("///"), "___");
        assert_eq!(Model::sanitize_fileref_name("Ok-Name_1.dat"), "Ok-Name_1.dat");
    }
}
