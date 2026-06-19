// Z-character text encoding — ZMSD §3.7.
//
// Encodes a (lower-cased, truncated) Rust string into the dictionary-resolution
// form: 4 bytes (6 Z-chars) for v3, 6 bytes (9 Z-chars) for v4+.
// Z-chars are packed three per 16-bit word, big-endian, with the terminator
// high bit (0x8000) set on the final word only.
//
// Scope: letters (A0) + A2 characters (shift-5 then A2 position).
// 10-bit ZSCII escapes are not implemented (real dictionary words are letters).

use super::{A0, A2};

/// Encode `text` to its dictionary-resolution Z-character form.
///
/// Returns 4 bytes (6 Z-chars) for v3, 6 bytes (9 Z-chars) for v4+.
/// The input is lower-cased before encoding. Characters longer than the
/// Z-char limit are truncated; shorter strings are padded with Z-char 5.
pub fn encode_word(text: &str, version: u8) -> Vec<u8> {
    let zchar_limit: usize = if version <= 3 { 6 } else { 9 };

    // Lower-case the input and build Z-char sequence.
    let lower = text.to_lowercase();
    let mut zchars: Vec<u8> = Vec::with_capacity(zchar_limit);

    'outer: for ch in lower.chars() {
        if zchars.len() >= zchar_limit {
            break;
        }
        let byte = ch as u8;

        // Try A0 (lowercase letters a-z).
        if let Some(pos) = A0.iter().position(|&b| b == byte) {
            zchars.push((pos + 6) as u8); // A0 Z-char = index + 6
            continue;
        }

        // Try A2 (digits, punctuation, etc.) — emits shift-5 then A2 Z-char.
        for (i, &b) in A2.iter().enumerate() {
            if b == byte && i >= 2 {
                // Need 2 Z-chars; only emit if both fit.
                if zchars.len() + 2 > zchar_limit {
                    break 'outer;
                }
                zchars.push(5); // shift to A2
                zchars.push((i + 6) as u8); // A2 Z-char = index + 6
                continue 'outer;
            }
        }

        // Character not encodable — skip (matches real-world dictionary scope).
    }

    // Pad to zchar_limit with Z-char 5.
    while zchars.len() < zchar_limit {
        zchars.push(5);
    }

    // Pack three Z-chars per 16-bit word.
    let word_count = zchar_limit / 3;
    let mut result = Vec::with_capacity(word_count * 2);

    for w in 0..word_count {
        let a = zchars[w * 3] as u16;
        let b = zchars[w * 3 + 1] as u16;
        let c = zchars[w * 3 + 2] as u16;
        let mut word: u16 = (a << 10) | (b << 5) | c;
        if w == word_count - 1 {
            word |= 0x8000; // terminator bit on final word only
        }
        result.push((word >> 8) as u8);
        result.push((word & 0xFF) as u8);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::header::tests_support::sample_story;
    use crate::memory::Memory;
    use crate::text::decode::decode_string;

    #[test]
    fn encodes_v3_word_to_four_bytes() {
        let enc = encode_word("sword", 3);
        assert_eq!(enc.len(), 4);
        // Terminator 0x8000 is the high bit of the MSB of the final 16-bit word,
        // which is enc[2] (the high byte of the second/last word).
        // Note: the brief specified enc[3] but that is the low byte; corrected here.
        assert_eq!(enc[2] & 0x80, 0x80); // terminator high bit on last word
    }

    #[test]
    fn encodes_v5_word_to_six_bytes() {
        let enc = encode_word("sword", 5);
        assert_eq!(enc.len(), 6);
        // Final word is bytes [4,5]; terminator high bit is enc[4] & 0x80.
        assert_eq!(enc[4] & 0x80, 0x80); // terminator high bit on last word
    }

    #[test]
    fn round_trip_v3() {
        // Encode "sword", write into a sample Memory, decode back, assert prefix.
        let enc = encode_word("sword", 3);
        let bytes = sample_story(3);
        let mut m = Memory::new(bytes).unwrap();
        // Write the 4-byte encoded form at 0x100.
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        // Decoded text must start with "sword" (padding Z-char 5 may decode as A2 shift, no output).
        assert!(decoded.starts_with("sword"), "decoded: {:?}", decoded);
    }

    #[test]
    fn round_trip_v5() {
        let enc = encode_word("sword", 5);
        let bytes = sample_story(5);
        let mut m = Memory::new(bytes).unwrap();
        m.write_word(0x100, ((enc[0] as u16) << 8) | enc[1] as u16);
        m.write_word(0x102, ((enc[2] as u16) << 8) | enc[3] as u16);
        m.write_word(0x104, ((enc[4] as u16) << 8) | enc[5] as u16);
        let (decoded, _) = decode_string(&m, 0x100);
        assert!(decoded.starts_with("sword"), "decoded: {:?}", decoded);
    }

    #[test]
    fn lowercases_input() {
        assert_eq!(encode_word("SWORD", 3), encode_word("sword", 3));
    }

    #[test]
    fn truncates_long_input() {
        // A word longer than 6 Z-chars for v3 must still produce 4 bytes.
        let enc = encode_word("abcdefghij", 3);
        assert_eq!(enc.len(), 4);
        assert_eq!(encode_word("abcdefghij", 3), encode_word("abcdef", 3));
    }

    #[test]
    fn pads_short_input() {
        // A very short word pads to the right length.
        let enc = encode_word("a", 3);
        assert_eq!(enc.len(), 4);
        // High byte of final word has the terminator bit set.
        assert_eq!(enc[2] & 0x80, 0x80);
    }
}
