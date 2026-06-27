//! Interpreter IFID derived from a Z-code story's header.

/// `ZCODE-<release>-<serial>-<checksum hex>` (release @0x02, serial @0x12, checksum @0x1C).
pub fn compute_ifid(story: &[u8]) -> String {
    if story.len() < 0x1E {
        return "ZCODE-INVALID".to_string();
    }
    let release = u16::from_be_bytes([story[0x02], story[0x03]]);
    let serial: String = story[0x12..0x18]
        .iter()
        .map(|&b| if b.is_ascii() && !b.is_ascii_control() { b as char } else { '-' })
        .collect();
    let checksum = u16::from_be_bytes([story[0x1C], story[0x1D]]);
    format!("ZCODE-{}-{}-{:04X}", release, serial, checksum)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story_with(release: u16, serial: &[u8; 6], checksum: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x40];
        b[0x02] = (release >> 8) as u8; b[0x03] = release as u8;
        b[0x12..0x18].copy_from_slice(serial);
        b[0x1C] = (checksum >> 8) as u8; b[0x1D] = checksum as u8;
        b
    }

    #[test]
    fn computes_zcode_ifid() {
        let s = story_with(42, b"871124", 0xABCD);
        assert_eq!(compute_ifid(&s), "ZCODE-42-871124-ABCD");
    }

    #[test]
    fn invalid_when_too_short() {
        assert_eq!(compute_ifid(&[0u8; 4]), "ZCODE-INVALID");
    }
}
