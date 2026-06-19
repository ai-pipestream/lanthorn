// ZSCII text decoding — ZMSD §3.2–§3.8.
//
// Each Z-string word packs three 5-bit Z-characters; the high bit (0x8000)
// marks the last word of the string. Three alphabets A0/A1/A2 cover
// lowercase, uppercase, and punctuation/digits. Shift Z-chars 4/5 (v3+)
// temporarily switch to A1/A2 for the next character only.

use crate::memory::Memory;
use super::{A0, A1, A2};

/// Decode a Z-encoded string starting at `addr`.
///
/// Returns `(decoded_string, address_past_end)`.
/// Abbreviation strings are decoded recursively (one level in practice).
/// Custom alphabet tables (header word 0x34) are not implemented — the
/// default table is always used.
pub fn decode_string(mem: &Memory, addr: u32) -> (String, u32) {
    // Collect all Z-chars first, recording where the string ends.
    let mut zchars: Vec<u8> = Vec::new();
    let mut pos = addr;
    loop {
        let word = mem.read_word(pos);
        let last = (word & 0x8000) != 0;
        pos += 2;
        zchars.push(((word >> 10) & 0x1F) as u8);
        zchars.push(((word >> 5) & 0x1F) as u8);
        zchars.push((word & 0x1F) as u8);
        if last {
            break;
        }
    }

    let end_addr = pos;
    let mut result = String::new();
    let mut i = 0;
    let mut alphabet: u8 = 0; // 0=A0, 1=A1, 2=A2
    let mut abbrev_pending: u8 = 0; // non-zero: waiting for abbrev index char

    while i < zchars.len() {
        let zc = zchars[i];
        i += 1;

        // Handle abbreviation index byte
        if abbrev_pending > 0 {
            let z = abbrev_pending;
            abbrev_pending = 0;
            let index = 32 * (z as u32 - 1) + zc as u32;
            let abbrev_base = mem.abbrev_table() as u32;
            let entry_addr = abbrev_base + 2 * index;
            let word_addr = mem.read_word(entry_addr) as u32;
            let byte_addr = word_addr * 2;
            let (abbrev_str, _) = decode_string(mem, byte_addr);
            result.push_str(&abbrev_str);
            // alphabet stays A0 after abbreviation
            continue;
        }

        match zc {
            0 => {
                result.push(' ');
                // space; any pending shift is consumed
                alphabet = 0;
            }
            1 | 2 | 3 => {
                abbrev_pending = zc;
                alphabet = 0;
            }
            4 => {
                // Shift to A1 for next character only (v3+)
                alphabet = 1;
            }
            5 => {
                // Shift to A2 for next character only (v3+)
                alphabet = 2;
            }
            zc => {
                if alphabet == 2 && zc == 6 {
                    // 10-bit ZSCII escape: next two Z-chars form the code
                    let hi = if i < zchars.len() { zchars[i] } else { 0 };
                    let lo = if i + 1 < zchars.len() { zchars[i + 1] } else { 0 };
                    i += 2;
                    let zscii = ((hi as u16) << 5) | (lo as u16);
                    result.push(zscii_to_char(zscii));
                    alphabet = 0;
                } else {
                    // Normal lookup: Z-char 6 → table index 0
                    let idx = (zc - 6) as usize;
                    let ch = match alphabet {
                        0 => A0[idx],
                        1 => A1[idx],
                        _ => A2[idx],
                    };
                    result.push(ch as char);
                    alphabet = 0;
                }
            }
        }
    }

    (result, end_addr)
}

/// Map a ZSCII value to a Rust `char` (ZMSD §3.8).
///
/// ZSCII 13 → '\n'. ASCII 32–126 are identity. Everything else maps to '?'.
pub(crate) fn zscii_to_char(zscii: u16) -> char {
    match zscii {
        13 => '\n',
        32..=126 => zscii as u8 as char,
        _ => '?',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;

    #[test]
    fn decodes_simple_lowercase_word() {
        // "abc" in A0: Z-chars 6,7,8. Word = (6<<10)|(7<<5)|8, top bit set (end).
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (6 << 10) | (7 << 5) | 8;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "abc");
        assert_eq!(end, 0x102);
    }

    #[test]
    fn decodes_space() {
        // Z-char 0 → ' '. Three spaces packed.
        let bytes = sample_story(3);
        let w: u16 = 0x8000; // all-zero Z-chars → three spaces
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, _end) = decode_string(&m, 0x100);
        assert_eq!(s, "   ");
    }

    #[test]
    fn decodes_uppercase_shift() {
        // "A" via shift-to-A1 (Z-char 4) followed by Z-char 6 (A1 index 0 = 'A').
        // Third Z-char in word is 5 (shift to A2) — since string ends there, no output.
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (4 << 10) | (6 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "A");
        assert_eq!(end, 0x102);
    }

    #[test]
    fn decodes_a2_digit() {
        // '0' via shift-to-A2 (Z-char 5) followed by Z-char 9 (A2 index 3 = '0').
        // A2: idx 0=escape, 1=\n, 2=space, 3='0', ...
        // Third Z-char is 5 (another shift) — string ends, no output.
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (5 << 10) | (9 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "0");
        assert_eq!(end, 0x102);
    }

    #[test]
    fn decodes_two_words() {
        // "abcdef" — two words, first without high bit, second with.
        let bytes = sample_story(3);
        let w1: u16 = (6 << 10) | (7 << 5) | 8; // a, b, c
        let w2: u16 = 0x8000 | (9 << 10) | (10 << 5) | 11; // d, e, f
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, "abcdef");
        assert_eq!(end, 0x104);
    }

    #[test]
    fn decodes_abbreviation() {
        // Abbreviation set 1 (Z-char 1), index 0 → expands to "abc".
        //
        // Abbreviation string at byte 0x0080: Z-chars [6,7,8] = "abc", terminated.
        //   word = 0x8000 | (6<<10) | (7<<5) | 8
        //
        // Abbreviation table base = 0x0040 (confirmed from sample_story header).
        // Entry for set 1, index 0: table_base + 2*( 32*(1-1)+0 ) = 0x0040.
        // Entry holds a WORD-address; decoder multiplies by 2 to get byte address.
        // To point at 0x0080, write word value 0x0040 at 0x0040.
        //
        // Main string at 0x0100: Z-chars [1, 0, 5], terminated.
        //   Z-char 1 = abbreviation trigger, Z-char 0 = index, Z-char 5 = shift (no output).
        //   word = 0x8000 | (1<<10) | (0<<5) | 5
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();

        // Write abbreviation string "abc" at 0x0080.
        let abbrev_word: u16 = 0x8000 | (6 << 10) | (7 << 5) | 8;
        m.write_word(0x0080, abbrev_word);

        // Write abbreviation table entry at 0x0040: word-address 0x0040 → byte 0x0080.
        m.write_word(0x0040, 0x0040);

        // Write main string at 0x0100: [1, 0, 5] with terminator.
        let main_word: u16 = 0x8000 | (1 << 10) | (0 << 5) | 5;
        m.write_word(0x0100, main_word);

        let (s, end) = decode_string(&m, 0x0100);
        assert_eq!(s, "abc");
        assert_eq!(end, 0x0102);
    }

    #[test]
    fn decodes_zscii_10bit_escape() {
        // ZSCII escape: A2 shift (5), then Z-char 6 (escape trigger),
        // then hi=1, lo=0 → ZSCII = (1<<5)|0 = 32 → ' ' (space via 10-bit path).
        // Pack Z-chars as: [5, 6, 1, 0, 5, 5] across two words.
        // Word 1: zchars [5, 6, 1]
        // Word 2: zchars [0, 5, 5] — the 0 and next are the hi/lo bytes consumed
        //         by the escape; trailing [5, 5] are unused shifts.
        // So total output: one space (' ', ZSCII 32).
        let bytes = sample_story(3);
        let w1: u16 = (5 << 10) | (6 << 5) | 1;
        let w2: u16 = 0x8000 | (0 << 10) | (5 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, " "); // ZSCII 32 = space
        assert_eq!(end, 0x104);
    }
}
