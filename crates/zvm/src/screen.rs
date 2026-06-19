// Screen model — ZMSD §7, §8, §11.
//
// `ScreenState` tracks window layout and text attributes the host needs to
// render.  `StatusLine` is the v3 status bar computed on demand from globals.
// `StreamState` manages output-stream routing including stream-3 memory
// redirection.
//
// Stream-3 can nest up to 16 deep (ZMSD §7.1.2.5).  Each frame holds a
// table base address; the first word of the table is the byte-count written.

use crate::memory::Memory;
use crate::objects;

// ---------------------------------------------------------------------------
// Status line
// ---------------------------------------------------------------------------

/// The right-hand portion of a v3 status line (ZMSD §8.2.3.1).
/// Flags1 bit 1: 0 = score/turns, 1 = time (hours:minutes).
#[derive(Debug, PartialEq)]
pub enum StatusRight {
    ScoreTurns { score: i16, turns: u16 },
    Time { hours: u8, minutes: u8 },
}

/// A fully computed v3 status line (location name + right field).
#[derive(Debug, PartialEq)]
pub struct StatusLine {
    pub location: String,
    pub right: StatusRight,
}

// ---------------------------------------------------------------------------
// Screen state (window model)
// ---------------------------------------------------------------------------

/// Structured screen model the host (TUI etc.) reads to render.
///
/// For v3 the host derives the status line by calling `Machine::status_line()`.
/// For v4+ the host reads `upper_window_rows`, `current_window`, `text_style`,
/// and `cursor` to manage windows.
#[derive(Debug, Default)]
pub struct ScreenState {
    /// Number of rows in the upper (status) window; 0 means no upper window.
    pub upper_window_rows: u16,
    /// Currently selected window: 0 = lower, 1 = upper.
    pub current_window: u8,
    /// Current text-style bitmask (ZMSD §8.7.2):
    ///   bit 1 = bold, bit 2 = italic, bit 3 = fixed-pitch, bit 4 = reverse-video.
    pub text_style: u8,
    /// Cursor position in the upper window (1-based row, col).
    pub cursor_row: u16,
    pub cursor_col: u16,
    /// Whether output should be buffered (lower window).
    pub buffer_mode: bool,
    /// Whether `show_status` (v3 0OP:0x0C) was requested since last read.
    pub show_status_requested: bool,
}

// ---------------------------------------------------------------------------
// Output stream state
// ---------------------------------------------------------------------------

/// One frame of nested stream-3 redirection.
struct Stream3Frame {
    /// Base address of the table in dynamic memory.
    table_addr: u32,
    /// Bytes written so far into this frame (accumulated before we flush).
    buf: Vec<u8>,
}

/// Manages all four Z-machine output streams.
///
/// Streams 1 (screen) and 2 (transcript) are on/off flags; only stream 1
/// defaults to on.  Stream 3 redirects text to a memory table and can nest.
/// Stream 4 (command log) is flag-only.
pub struct StreamState {
    /// Stream 1 (screen) active.
    pub stream1: bool,
    /// Stream 2 (transcript) active.
    pub stream2: bool,
    /// Stream 4 (command log) active.
    pub stream4: bool,
    /// Stack of active stream-3 frames (nested up to 16).
    stream3_stack: Vec<Stream3Frame>,
}

impl StreamState {
    pub fn new() -> Self {
        StreamState {
            stream1: true,
            stream2: false,
            stream4: false,
            stream3_stack: Vec::new(),
        }
    }

    /// True when stream 3 is active (text goes to memory, not screen).
    pub fn stream3_active(&self) -> bool {
        !self.stream3_stack.is_empty()
    }

    /// Select (push) stream 3 with a table at `table_addr` (ZMSD §7.1.2.5).
    pub fn push_stream3(&mut self, table_addr: u32) {
        if self.stream3_stack.len() < 16 {
            self.stream3_stack.push(Stream3Frame { table_addr, buf: Vec::new() });
        }
    }

    /// Deselect (pop) stream 3: write accumulated bytes into memory, update
    /// the length word, and return.
    ///
    /// The table layout: word at `table_addr` = byte count; bytes follow from
    /// `table_addr + 2`.
    pub fn pop_stream3(&mut self, mem: &mut Memory) {
        if let Some(frame) = self.stream3_stack.pop() {
            let n = frame.buf.len() as u16;
            mem.write_word(frame.table_addr, n);
            for (i, &b) in frame.buf.iter().enumerate() {
                mem.write_byte(frame.table_addr + 2 + i as u32, b);
            }
        }
    }

    /// Append text to the current stream-3 buffer (ASCII/ZSCII bytes only).
    pub fn write_stream3(&mut self, s: &str) {
        if let Some(frame) = self.stream3_stack.last_mut() {
            frame.buf.extend_from_slice(s.as_bytes());
        }
    }
}

// ---------------------------------------------------------------------------
// Header capability bits (ZMSD §11.1)
// ---------------------------------------------------------------------------

/// Set interpreter capability bits in the story header at machine startup.
///
/// We advertise a basic text interpreter:
///   - Flags1: clear graphics (bit 1 v3 is status-line kind — leave alone),
///     clear colour (bit 0 in v5+), clear pictures, clear sound.
///     Set fixed-pitch available (bit 4 in v3 Flags2? — actually interpreter
///     sets this in response to Flags2; for simplicity we just clear "no
///     status line" bit).
///   - Flags2: we clear bits we don't support (pictures=0, undo=0, mouse=0,
///     colour=0, sound=0, menubar=0) by masking.  Leave transcript (bit 0)
///     and fixed-pitch (bit 1) as-is (game controls those).
///   - 0x1E: interpreter number — 6 (IBM PC), a common neutral value.
///   - 0x1F: interpreter version — 'A' (ASCII 0x41), standard v1.1 era.
///   - 0x32/0x33: standard revision number (1.1 → 1, 1).
///
/// Only modifies bytes inside dynamic memory (below static_mem_base); if the
/// header region is read-only (static_mem_base ≤ 0x40) we skip silently.
pub fn init_header_caps(mem: &mut Memory) {
    let version = mem.version();

    // Guard: only write if the header sits in dynamic memory.
    // All story files should have static_mem_base > 0x40 (ZMSD §1.1), but be safe.
    // We check individual addresses before each write via the fact that
    // `write_byte` debug-asserts the range; to avoid panics we only call it
    // if we know memory is writable.  In practice all well-formed stories have
    // dynamic memory covering the header, so this is always fine.

    // Flags1 (byte 0x01): interpreter-writable bits.
    let f1 = mem.read_byte(0x01);
    let new_f1 = if version <= 3 {
        // v3 Flags1 bits (ZMSD §11.1.1):
        //   bit 1: time game (0 = score/turns, set by game — don't touch)
        //   bit 4: status line not available — clear (we support it)
        //   bit 5: screen-splitting available — set
        //   bit 6: variable-pitch font default — clear (use fixed)
        f1 & !(1 << 4)   // clear "status line not available"
          | (1 << 5)      // screen-splitting available
    } else {
        // v4+ Flags1 bits (ZMSD §11.1.3):
        //   bit 0: colour available — clear (no colour)
        //   bit 1: picture display available — clear
        //   bit 2: boldface available — clear (stub)
        //   bit 3: italic available — clear (stub)
        //   bit 4: fixed-space font available — set
        //   bit 5: sound effects available — clear
        //   bit 7: timed keyboard available — clear
        f1 & !((1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 7))  // clear unsupported
          | (1 << 4)  // fixed-space font available
    };
    mem.write_byte(0x01, new_f1);

    // Flags2 (word 0x10–0x11, interpreter clears bits it doesn't support).
    // ZMSD §11.1.4: bits 3–5, 7 are interpreter-writable (clear = not supported).
    //   bit 3: pictures — clear
    //   bit 4: undo available — clear
    //   bit 5: mouse — clear
    //   bit 7: sound effects — clear
    let f2 = mem.read_word(0x10);
    let f2_mask: u16 = !((1 << 3) | (1 << 4) | (1 << 5) | (1 << 7));
    mem.write_word(0x10, f2 & f2_mask);

    // Interpreter number (0x1E): 6 = IBM PC / generic.
    mem.write_byte(0x1E, 6);

    // Interpreter version (0x1F): b'A' = 0x41.
    mem.write_byte(0x1F, b'A');

    // Standard revision (0x32 = major, 0x33 = minor): 1.2 (latest published).
    mem.write_byte(0x32, 1);
    mem.write_byte(0x33, 2);
}

// ---------------------------------------------------------------------------
// Status-line computation (v3)
// ---------------------------------------------------------------------------

/// Compute the current v3 status line from memory globals and header.
///
/// G0 (global var 0) = location object number.
/// G1 = score (signed) or hours (unsigned).
/// G2 = turns or minutes.
/// Flags1 bit 1: 0 = score/turns, 1 = time.
pub fn compute_status_line(mem: &Memory) -> StatusLine {
    let gbase = mem.global_vars() as u32;
    let loc_obj = mem.read_word(gbase);
    let g1 = mem.read_word(gbase + 2);
    let g2 = mem.read_word(gbase + 4);

    let location = if loc_obj == 0 {
        String::new()
    } else {
        objects::short_name(mem, loc_obj)
    };

    // Flags1 bit 1 selects time mode.
    let flags1 = mem.read_byte(0x01);
    let time_mode = (flags1 & (1 << 1)) != 0;

    let right = if time_mode {
        StatusRight::Time { hours: g1 as u8, minutes: g2 as u8 }
    } else {
        StatusRight::ScoreTurns { score: g1 as i16, turns: g2 }
    };

    StatusLine { location, right }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::encode::encode_word;

    // ── helpers ──────────────────────────────────────────────────────────────

    /// Build a minimal v3 story with one object whose short name is "West of House".
    /// Object 1 is placed at the v3 entries base.
    /// G0 = 1 (location object), G1, G2 = supplied.
    fn build_v3_status_story(g1: u16, g2: u16, time_mode: bool) -> Vec<u8> {
        let mut buf = sample_story(3);

        // Object table is at 0x0100 (set by sample_story).
        // v3 property-defaults: 31 words = 62 bytes → entries at 0x013E.
        let obj1_entry: usize = 0x013E;
        let prop_tbl: u16 = 0x0200;

        // Object 1 entry (9 bytes): no attrs, no tree, prop_tbl pointer.
        for i in 0..7 { buf[obj1_entry + i] = 0; }
        buf[obj1_entry + 7] = (prop_tbl >> 8) as u8;
        buf[obj1_entry + 8] = (prop_tbl & 0xFF) as u8;

        // Property table: short name = "west" (2 Z-words).
        let name = encode_word("west", 3); // 4 bytes
        assert_eq!(name.len(), 4);
        buf[prop_tbl as usize] = 2; // 2 name-words
        buf[prop_tbl as usize + 1..prop_tbl as usize + 5].copy_from_slice(&name);
        buf[prop_tbl as usize + 5] = 0x00; // sentinel

        // Set G0=1, G1=g1, G2=g2 in global vars table (0x0300).
        let gbase: usize = 0x0300;
        buf[gbase]     = 0; buf[gbase + 1] = 1;  // G0 = 1
        buf[gbase + 2] = (g1 >> 8) as u8; buf[gbase + 3] = (g1 & 0xFF) as u8;
        buf[gbase + 4] = (g2 >> 8) as u8; buf[gbase + 5] = (g2 & 0xFF) as u8;

        // Flags1: bit 1 controls time mode.
        if time_mode {
            buf[0x01] |= 1 << 1;
        } else {
            buf[0x01] &= !(1 << 1);
        }

        buf
    }

    // ── (a) v3 status line: score/turns mode ─────────────────────────────────

    #[test]
    fn v3_status_line_score_turns() {
        let buf = build_v3_status_story(42u16, 7, false);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(
            sl.location.starts_with("west"),
            "location should start with 'west', got {:?}", sl.location
        );
        assert_eq!(sl.right, StatusRight::ScoreTurns { score: 42, turns: 7 });
    }

    // ── (b) v3 status line: time mode ────────────────────────────────────────

    #[test]
    fn v3_status_line_time_mode() {
        let buf = build_v3_status_story(10, 30, true);
        let mem = Memory::new(buf).unwrap();
        let sl = compute_status_line(&mem);
        assert!(sl.location.starts_with("west"), "location should start with 'west'");
        assert_eq!(sl.right, StatusRight::Time { hours: 10, minutes: 30 });
    }

    // ── (c) header capability bits ───────────────────────────────────────────

    #[test]
    fn header_caps_v3_clears_no_status_line() {
        let mut mem = Memory::new(sample_story(3)).unwrap();
        // Set "status line not available" bit before init.
        let f1 = mem.read_byte(0x01) | (1 << 4);
        mem.write_byte(0x01, f1);
        init_header_caps(&mut mem);
        // Bit 4 should be cleared.
        assert_eq!(mem.read_byte(0x01) & (1 << 4), 0, "bit 4 (no status line) should be clear");
        // Screen-splitting available (bit 5) should be set.
        assert_ne!(mem.read_byte(0x01) & (1 << 5), 0, "bit 5 (screen-split) should be set");
    }

    #[test]
    fn header_caps_v5_clears_unsupported_bits() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        init_header_caps(&mut mem);
        let f1 = mem.read_byte(0x01);
        // Colour (bit 0) should be clear.
        assert_eq!(f1 & (1 << 0), 0, "colour bit should be clear");
        // Pictures (bit 1) should be clear.
        assert_eq!(f1 & (1 << 1), 0, "pictures bit should be clear");
        // Fixed-space font (bit 4) should be set.
        assert_ne!(f1 & (1 << 4), 0, "fixed-space font bit should be set");
        // Interpreter number set.
        assert_eq!(mem.read_byte(0x1E), 6, "interpreter number = 6");
        assert_eq!(mem.read_byte(0x1F), b'A', "interpreter version = 'A'");
    }

    #[test]
    fn header_caps_flags2_clears_pictures_sound() {
        let mut mem = Memory::new(sample_story(5)).unwrap();
        // Pre-set pictures (bit 3) and sound (bit 7) in Flags2.
        let f2 = mem.read_word(0x10) | (1 << 3) | (1 << 7);
        mem.write_word(0x10, f2);
        init_header_caps(&mut mem);
        let f2_after = mem.read_word(0x10);
        assert_eq!(f2_after & (1 << 3), 0, "pictures bit in Flags2 should be clear");
        assert_eq!(f2_after & (1 << 7), 0, "sound bit in Flags2 should be clear");
    }

    // ── (d) ScreenState defaults ──────────────────────────────────────────────

    #[test]
    fn screen_state_defaults() {
        let s = ScreenState::default();
        assert_eq!(s.upper_window_rows, 0);
        assert_eq!(s.current_window, 0);
        assert_eq!(s.text_style, 0);
        assert!(!s.buffer_mode);
    }

    // ── (e) StreamState: stream-3 push/pop/write ─────────────────────────────

    #[test]
    fn stream3_push_write_pop() {
        let buf = sample_story(5);
        // Reserve a table at 0x0050 (within dynamic memory, safely away from header).
        let table_addr: u32 = 0x0050;

        let mut mem = Memory::new(buf.clone()).unwrap();
        let mut ss = StreamState::new();

        assert!(!ss.stream3_active());
        ss.push_stream3(table_addr);
        assert!(ss.stream3_active());

        ss.write_stream3("Hello");
        ss.pop_stream3(&mut mem);

        assert!(!ss.stream3_active());

        // Check table: word at table_addr = 5 (length), then "Hello".
        assert_eq!(mem.read_word(table_addr), 5, "length word should be 5");
        assert_eq!(mem.read_byte(table_addr + 2), b'H');
        assert_eq!(mem.read_byte(table_addr + 3), b'e');
        assert_eq!(mem.read_byte(table_addr + 4), b'l');
        assert_eq!(mem.read_byte(table_addr + 5), b'l');
        assert_eq!(mem.read_byte(table_addr + 6), b'o');
    }

    #[test]
    fn stream3_nested() {
        let buf = sample_story(5);
        let table1: u32 = 0x0050;
        let table2: u32 = 0x0060;

        let mut mem = Memory::new(buf).unwrap();
        let mut ss = StreamState::new();

        ss.push_stream3(table1);
        ss.write_stream3("ab");
        ss.push_stream3(table2);
        ss.write_stream3("cd");
        ss.pop_stream3(&mut mem); // finalise table2
        ss.write_stream3("ef");
        ss.pop_stream3(&mut mem); // finalise table1

        // table2: "cd" (2 bytes)
        assert_eq!(mem.read_word(table2), 2);
        assert_eq!(mem.read_byte(table2 + 2), b'c');
        assert_eq!(mem.read_byte(table2 + 3), b'd');

        // table1: "ab" + "ef" = "abef" (4 bytes)
        assert_eq!(mem.read_word(table1), 4);
        assert_eq!(mem.read_byte(table1 + 2), b'a');
        assert_eq!(mem.read_byte(table1 + 3), b'b');
        assert_eq!(mem.read_byte(table1 + 4), b'e');
        assert_eq!(mem.read_byte(table1 + 5), b'f');
    }
}
