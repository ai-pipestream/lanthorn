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
/// Custom alphabet tables (header word 0x34, v5+) override the default A0/A1/A2
/// glyphs when present; otherwise the default table is used.
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

    // v5+ custom alphabet table (header 0x34). When present, glyph rows come from
    // the table (78 ZSCII bytes: A0 0..26, A1 26..52, A2 52..78). The A2 specials
    // (position 0 = escape, 1 = newline) keep their built-in handling.
    let custom = if mem.version() >= 5 {
        let p = mem.read_word(0x34) as u32;
        (p != 0).then_some(p)
    } else {
        None
    };

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
            1..=3 => {
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
                    // Consult the story's custom Unicode table first (ZMSD §3.8.5.4);
                    // fall back to the default ZSCII table.
                    let ch = mem.unicode_char(zscii).unwrap_or_else(|| zscii_to_char(zscii));
                    result.push(ch);
                    alphabet = 0;
                } else {
                    // Normal lookup: Z-char 6 → table index 0.
                    let idx = (zc - 6) as usize;
                    // A2 newline (idx 1) is special and is never taken from a custom
                    // table; A2 escape (idx 0) is already handled above.
                    let ch = match custom {
                        Some(tbl) if !(alphabet == 2 && idx == 1) => {
                            let row = alphabet as u32;
                            mem.read_byte(tbl + row * 26 + idx as u32)
                        }
                        _ => match alphabet {
                            0 => A0[idx],
                            1 => A1[idx],
                            _ => A2[idx],
                        },
                    };
                    result.push(ch as char);
                    alphabet = 0;
                }
            }
        }
    }

    (result, end_addr)
}

/// Default Unicode translation table for ZSCII 155–223 (ZMSD §3.8.5).
const UNICODE_TABLE: [char; 69] = [
    'ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '»', '«', // 155–163
    'ë', 'ï', 'ÿ', 'Ë', 'Ï', 'á', 'é', 'í', 'ó',  // 164–172
    'ú', 'ý', 'Á', 'É', 'Í', 'Ó', 'Ú', 'Ý', 'à',  // 173–181
    'è', 'ì', 'ò', 'ù', 'À', 'È', 'Ì', 'Ò', 'Ù',  // 182–190
    'â', 'ê', 'î', 'ô', 'û', 'Â', 'Ê', 'Î', 'Ô',  // 191–199
    'Û', 'å', 'Å', 'ø', 'Ø', 'ã', 'ñ', 'õ', 'Ã',  // 200–208
    'Ñ', 'Õ', 'æ', 'Æ', 'ç', 'Ç', 'þ', 'ð', 'Þ',  // 209–217
    'Ð', '£', 'œ', 'Œ', '¡', '¿',                  // 218–223
];

/// Map a ZSCII value to a Rust `char` (ZMSD §3.8).
///
/// ZSCII 13 → '\n'. ASCII 32–126 are identity.
/// ZSCII 155–223 → default Unicode translation table (ZMSD §3.8.5).
/// Everything else maps to '?'.
pub(crate) fn zscii_to_char(zscii: u16) -> char {
    match zscii {
        13 => '\n',
        32..=126 => zscii as u8 as char,
        155..=223 => UNICODE_TABLE[(zscii - 155) as usize],
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
        // '0' via shift-to-A2 (Z-char 5) followed by Z-char 8 (A2 index 2 = '0').
        // A2: idx 0=escape, 1=\n, 2='0', 3='1', ...
        // Third Z-char is 5 (another shift) — string ends, no output.
        let bytes = sample_story(3);
        let w: u16 = 0x8000 | (5 << 10) | (8 << 5) | 5;
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
    fn custom_unicode_table_overrides_default_zscii_155() {
        // Build a story with a header-extension table (at 0x0200) that points
        // to a Unicode translation table (at 0x0210).
        // Header word 0x36 = address of header-extension table (0x0200).
        // Header-ext word 0 = number of words in table = 3.
        // Header-ext word 3 (byte offset 6) = address of Unicode table (0x0210).
        // Unicode table: byte 0 = N=1, then 1 word = U+00A9 (©).
        // This maps ZSCII 155 → '©' (overriding default 'ä').
        //
        // Encode ZSCII 155 via the 10-bit escape: A2 shift (5), Z-char 6 (escape),
        // hi = 155 >> 5 = 4, lo = 155 & 31 = 27.
        // Pack into two words:
        //   w1 = [5, 6, 4]
        //   w2 (end) = [27, 5, 5]
        let mut bytes = sample_story(5); // v5 has header-extension table
        // header-ext table address at byte 0x36
        bytes[0x36] = 0x02; bytes[0x37] = 0x00; // 0x0200
        // header-ext table at 0x0200: word 0 = 3 (table has 3 words)
        bytes[0x0200] = 0x00; bytes[0x0201] = 0x03; // word 0: length=3
        bytes[0x0202] = 0x00; bytes[0x0203] = 0x00; // word 1: unused
        bytes[0x0204] = 0x00; bytes[0x0205] = 0x00; // word 2: unused
        bytes[0x0206] = 0x02; bytes[0x0207] = 0x10; // word 3: Unicode table at 0x0210
        // Unicode translation table at 0x0210: N=1, then word U+00A9 (©)
        bytes[0x0210] = 0x01;                        // N=1
        bytes[0x0211] = 0x00; bytes[0x0212] = 0xA9; // U+00A9 = ©
        let mut m = Memory::new(bytes).unwrap();
        // Encode ZSCII 155 via 10-bit escape
        let w1: u16 = (5 << 10) | (6 << 5) | 4;
        let w2: u16 = 0x8000 | (27 << 10) | (5 << 5) | 5;
        m.write_word(0x0100, w1);
        m.write_word(0x0102, w2);
        let (s, _end) = decode_string(&m, 0x0100);
        assert_eq!(s, "©", "custom Unicode table should map ZSCII 155 to © not ä");
    }

    #[test]
    fn custom_alphabet_table_overrides_default_glyphs() {
        // v5 story with header 0x34 -> a custom 78-byte alphabet table whose A0[0]='z'.
        let mut bytes = sample_story(5);
        let tbl: usize = 0x0200;
        bytes[0x34] = (tbl >> 8) as u8;
        bytes[0x35] = (tbl & 0xFF) as u8;
        let a0 = b"zbcdefghijklmnopqrstuvwxyz";
        let a1 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let a2 = b"\x00\n0123456789.,!?_#'\"/\\-:()";
        for (i, &c) in a0.iter().enumerate() { bytes[tbl + i] = c; }
        for (i, &c) in a1.iter().enumerate() { bytes[tbl + 26 + i] = c; }
        for (i, &c) in a2.iter().enumerate() { bytes[tbl + 52 + i] = c; }
        let mut m = Memory::new(bytes).unwrap();
        // One A0 Z-char 6 (the first A0 letter), padded with shifts (no output).
        let w: u16 = 0x8000 | (6 << 10) | (5 << 5) | 5;
        m.write_word(0x0100, w);
        let (s, _end) = decode_string(&m, 0x0100);
        assert_eq!(s, "z", "custom A0[0] should decode to 'z' not the default 'a'");
    }

    #[test]
    fn zscii_extended_chars_decode() {
        assert_eq!(zscii_to_char(155), 'ä');
        assert_eq!(zscii_to_char(161), 'ß');
        assert_eq!(zscii_to_char(219), '£');
        assert_eq!(zscii_to_char(220), 'œ');
        assert_eq!(zscii_to_char(223), '¿');
        // boundary: just outside the table
        assert_eq!(zscii_to_char(154), '?');
        assert_eq!(zscii_to_char(224), '?');
    }

    #[test]
    fn decodes_digits_and_punctuation() {
        // Regression: A2 table must NOT have a space at index 2.
        // Per ZMSD §3.5.3, fixed A2: idx 2='0', idx 11='9', idx 12='.', idx 18='\''.
        // Z-char 5 = A2 shift; the following Z-char selects the character.
        // Period ('.') = A2 idx 12 = Z-char 18.
        // '9'         = A2 idx 11 = Z-char 17.
        // Apostrophe  = A2 idx 18 = Z-char 24.
        //
        // Word 1: [5, 18, 5]  → '.' then shift-A2
        // Word 2: [17, 5, 24] → '9' then shift-A2 then '\''? No: after shift+char,
        //   alphabet resets. So: [5, 18, 5] → '.', shift pending; [17] → wrong.
        //
        // Use three words for clarity:
        // Word 1 (no-end): Z-chars [5, 18, 5]  → period, then shift-A2 starts
        // Word 2 (no-end): Z-chars [17, 5, 24] → '9', shift-A2, '\''
        // Word 3 (end):    Z-chars [5, 5, 5]   → three unused shifts (no chars)
        //
        // Actually pack more carefully — each Z-char needs its own shift:
        // Word 1: [5, 18, 5]   → '.', A2-shift for next char
        // Word 2: [17, 5, 24]  → '9', A2-shift, '\''  ← shift is consumed by 24
        // Wait: after outputting '.', alphabet resets to 0.
        // [5, 18, 5]:  5=A2-shift, 18='.' output, 5=A2-shift again
        // [17, ...]    but alphabet=2 so 17 → A2 idx 11 = '9'; alphabet resets
        // [5, 24, 5]   5=A2-shift, 24='\'' output, 5=unused-shift
        // So three words:
        //   w1 = [5, 18, 5]
        //   w2 = [17, 5, 24]
        //   w3 (end) = [5, 5, 5]
        // Result: '.', '9', '\''
        let bytes = sample_story(3);
        let w1: u16 = (5 << 10) | (18 << 5) | 5;
        let w2: u16 = (17 << 10) | (5 << 5) | 24;
        let w3: u16 = 0x8000 | (5 << 10) | (5 << 5) | 5;
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, w1);
        m.write_word(0x102, w2);
        m.write_word(0x104, w3);
        let (s, end) = decode_string(&m, 0x100);
        assert_eq!(s, ".9'", "period should be '.' not '9', apostrophe should be '\\'' not '#'");
        assert_eq!(end, 0x106);
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
