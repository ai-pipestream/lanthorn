//! Aux ("global state") persistence for zvm-cli: a per-story `<stem>.aux` file
//! holding the v5 save/restore-table map (`Machine::aux_data`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"ZAUX";
const VERSION: u8 = 1;

#[derive(Debug, PartialEq)]
pub enum AuxError {
    BadVersion,
    Truncated,
}

/// Aux file path: `default.aux` inside the per-game directory.
pub fn aux_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.aux")
}

/// Encode the aux-table map as the length-prefixed `ZAUX` v1 format.
pub fn encode_aux(map: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&(map.len() as u32).to_le_bytes());
    for (name, data) in map {
        let nb = name.as_bytes();
        out.extend_from_slice(&(nb.len() as u32).to_le_bytes());
        out.extend_from_slice(nb);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

fn take<'a>(b: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], AuxError> {
    let end = pos.checked_add(n).ok_or(AuxError::Truncated)?;
    let s = b.get(*pos..end).ok_or(AuxError::Truncated)?;
    *pos = end;
    Ok(s)
}

fn take_u32(b: &[u8], pos: &mut usize) -> Result<usize, AuxError> {
    let s = take(b, pos, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
}

/// Decode an aux buffer. Tolerant for back-compat (SQ-0300): a buffer bearing
/// the `ZAUX` magic is parsed as v1 (rejecting bad version/truncation); a buffer
/// WITHOUT the magic is treated as the legacy app format (no magic, big-endian,
/// u16 name length) and parsed leniently, so old `default.aux` files still load.
pub fn decode_aux(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, AuxError> {
    if bytes.len() < 4 || &bytes[..4] != MAGIC {
        return Ok(decode_legacy(bytes));
    }
    let mut pos = 4; // magic already matched
    if take(bytes, &mut pos, 1)?[0] != VERSION {
        return Err(AuxError::BadVersion);
    }
    let count = take_u32(bytes, &mut pos)?;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let nlen = take_u32(bytes, &mut pos)?;
        let name = String::from_utf8(take(bytes, &mut pos, nlen)?.to_vec())
            .map_err(|_| AuxError::Truncated)?;
        let dlen = take_u32(bytes, &mut pos)?;
        let data = take(bytes, &mut pos, dlen)?.to_vec();
        map.insert(name, data);
    }
    Ok(map)
}

/// Parse the legacy app format (no magic, big-endian, u16 name length).
/// Tolerant: any truncation/overflow yields whatever parsed so far.
fn decode_legacy(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let end = p.checked_add(n)?;
        let s = bytes.get(*p..end)?;
        *p = end;
        Some(s)
    };
    let count = match take(&mut p, 4) {
        Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        None => return out,
    };
    for _ in 0..count {
        let nl = match take(&mut p, 2) { Some(b) => u16::from_be_bytes([b[0], b[1]]) as usize, None => break };
        let name = match take(&mut p, nl) { Some(b) => String::from_utf8_lossy(b).into_owned(), None => break };
        let dl = match take(&mut p, 4) { Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize, None => break };
        let data = match take(&mut p, dl) { Some(b) => b.to_vec(), None => break };
        out.insert(name, data);
    }
    out
}

#[cfg(test)]
mod path_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn aux_path_is_default_aux_in_the_game_dir() {
        let gd = Path::new("/data/Zork1.z5");
        assert_eq!(aux_path(gd), PathBuf::from("/data/Zork1.z5/default.aux"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact `ZAUX` v1 bytes both hosts must emit (matches app's literal).
    const ZAUX_BYTES: &[u8] = &[
        b'Z', b'A', b'U', b'X', 0x01, // magic + version
        0x01, 0x00, 0x00, 0x00, // count = 1 (LE)
        0x02, 0x00, 0x00, 0x00, b'A', b'B', // name_len 2 (LE) + "AB"
        0x04, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF, // data_len 4 (LE) + data
    ];

    fn cross_host_sample() -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        m.insert("AB".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        m
    }

    /// Legacy app format encoder (no magic, big-endian, u16 name length).
    fn encode_legacy(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(table.len() as u32).to_be_bytes());
        for (name, data) in table {
            let nb = name.as_bytes();
            out.extend_from_slice(&(nb.len() as u16).to_be_bytes());
            out.extend_from_slice(nb);
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(data);
        }
        out
    }

    #[test]
    fn codec_round_trips() {
        let mut m = BTreeMap::new();
        m.insert("FORM".to_string(), vec![1, 2, 3]);
        m.insert("memo".to_string(), Vec::new());
        m.insert("ünïcode".to_string(), vec![9]);
        let bytes = encode_aux(&m);
        assert_eq!(decode_aux(&bytes).unwrap(), m);
    }

    #[test]
    fn encodes_canonical_zaux_bytes() {
        // Byte-identity with app: both hosts assert against this same literal.
        assert_eq!(encode_aux(&cross_host_sample()), ZAUX_BYTES);
    }

    #[test]
    fn decodes_zaux_from_other_host() {
        assert_eq!(decode_aux(ZAUX_BYTES).unwrap(), cross_host_sample());
    }

    #[test]
    fn decodes_legacy_app_format() {
        // Back-compat: an app-written (no-magic/BE/u16) file still loads.
        let mut m = BTreeMap::new();
        m.insert("FORM".to_string(), vec![1, 2, 3]);
        m.insert(String::new(), Vec::new());
        assert_eq!(decode_aux(&encode_legacy(&m)).unwrap(), m);
    }

    #[test]
    fn decode_rejects_bad_input() {
        // A real ZAUX buffer with a bad version or truncation is still rejected.
        let mut good = encode_aux(&BTreeMap::new());
        good[4] = 99; // version byte
        assert!(matches!(decode_aux(&good), Err(AuxError::BadVersion)));
        assert!(matches!(
            decode_aux(b"ZAUX\x01\x00\x00\x00\x05"),
            Err(AuxError::Truncated)
        ));
        // Non-ZAUX bytes are treated as the tolerant legacy format, not an error.
        assert!(decode_aux(b"XXXX").unwrap().is_empty());
    }
}
