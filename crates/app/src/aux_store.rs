//! Auxiliary save-data codec + global-file backend (v5 `save/restore table`).
//! See docs/superpowers/specs/2026-06-26-aux-save-data-design.md.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Encode the aux table as a compact length-prefixed binary blob:
/// `u32 count` then per entry `u16 name_len, name, u32 data_len, data`
/// (all big-endian). Deterministic because the input is a BTreeMap.
pub fn encode_aux(table: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
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

/// Decode `encode_aux` output. Tolerant: any truncation/overflow yields whatever
/// was parsed so far (empty for non-aux bytes), never panics or errors.
pub fn decode_aux(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*p..*p + n)?;
        *p += n;
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

/// `<game_dir>/default.aux` (SQ-0284). The aux table is the game's singleton
/// side data, stored under the per-game directory keyed by story filename.
pub fn aux_path(game_dir: &Path) -> PathBuf {
    game_dir.join("default.aux")
}

/// Read the per-game global aux file (empty map if absent or unreadable).
pub fn read_global_aux(game_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    match std::fs::read(aux_path(game_dir)) {
        Ok(bytes) => decode_aux(&bytes),
        Err(_) => BTreeMap::new(),
    }
}

/// Write the per-game global aux file (creating `game_dir` if needed).
pub fn write_global_aux(game_dir: &Path, table: &BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
    std::fs::create_dir_all(game_dir)?;
    std::fs::write(aux_path(game_dir), encode_aux(table))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BTreeMap<String, Vec<u8>> {
        let mut m = BTreeMap::new();
        m.insert("AB".to_string(), vec![0xDE, 0xAD, 0xBE, 0xEF]);
        m.insert("".to_string(), vec![]); // empty key + empty value
        m
    }

    #[test]
    fn codec_round_trips() {
        let m = sample();
        assert_eq!(decode_aux(&encode_aux(&m)), m);
    }

    #[test]
    fn decode_tolerates_garbage() {
        assert!(decode_aux(b"\xff\xff\xffnonsense").is_empty());
        assert!(decode_aux(&[]).is_empty());
    }

    #[test]
    fn aux_path_is_default_aux_in_game_dir() {
        let dir = Path::new("/tmp/saves/Zork1.z5");
        let p = aux_path(dir);
        assert_eq!(p, PathBuf::from("/tmp/saves/Zork1.z5/default.aux"));
        assert_eq!(p.parent(), Some(dir), "stays in the game dir");
    }

    #[test]
    fn global_file_round_trips() {
        let dir = std::env::temp_dir().join(format!("babelmap-aux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_global_aux(&dir).is_empty(), "absent file → empty");
        write_global_aux(&dir, &sample()).unwrap();
        assert_eq!(read_global_aux(&dir), sample());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
