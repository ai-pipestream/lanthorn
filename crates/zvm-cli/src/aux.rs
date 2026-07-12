//! Aux ("global state") persistence for zvm-cli: a per-story `<stem>.aux` file
//! holding the v5 save/restore-table map (`Machine::aux_data`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"ZAUX";
const VERSION: u8 = 1;

#[derive(Debug, PartialEq)]
pub enum AuxError {
    BadMagic,
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

/// Decode a `ZAUX` v1 buffer; rejects bad magic/version or truncation.
pub fn decode_aux(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, AuxError> {
    let mut pos = 0;
    if take(bytes, &mut pos, 4)? != MAGIC {
        return Err(AuxError::BadMagic);
    }
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
    fn decode_rejects_bad_input() {
        assert!(matches!(decode_aux(b"XXXX"), Err(AuxError::BadMagic)));
        let mut good = encode_aux(&BTreeMap::new());
        good[4] = 99; // version byte
        assert!(matches!(decode_aux(&good), Err(AuxError::BadVersion)));
        assert!(matches!(
            decode_aux(b"ZAUX\x01\x00\x00\x00\x05"),
            Err(AuxError::Truncated)
        ));
    }
}
