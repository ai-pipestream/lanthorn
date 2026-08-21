//! `BPal` — the pre-baked adaptive-palette table (a converter extension, NOT
//! part of the Blorb standard).
//!
//! Blorb §11.3 says a picture listed in `APal` carries a placeholder palette and
//! must be plotted with the "Current Palette" — the palette of the most recently
//! drawn non-adaptive picture. Infocom's own blorbs (Zork Zero, Arthur) ship a
//! `BPal` chunk beside `APal` in which the converter has already *performed*
//! that computation for every combination and stored the finished pictures as
//! extra `Pict` resources in a high id block. The chunk is the index into that
//! block: a flat array of big-endian u32 triples,
//!
//! ```text
//! (background_picture_id, adaptive_picture_id, baked_picture_id)
//! ```
//!
//! one row per (non-adaptive picture, adaptive picture) pair. Zork Zero's is
//! 224x172 = 38528 rows; Arthur's is 134x3 = 402.
//!
//! Nothing in lanthorn's *runtime* uses this — [`crate::Blorb`] deliberately
//! ignores `BPal` and the app computes §11.3 live, so a container that lacks the
//! table (every non-Infocom blorb) behaves identically. It is decoded here
//! because it is an independent oracle for that live computation: the app's
//! adaptive test replays every row and checks our splice against the converter's
//! answer. Kept a free function over the raw file bytes rather than a `Blorb`
//! field so the standard parse path pays nothing for a chunk it ignores.

/// One `BPal` row: plotting adaptive picture `adaptive` while `background` holds
/// the Current Palette should produce exactly Pict `baked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteBake {
    /// Pict number of the non-adaptive picture whose palette is current.
    pub background: u32,
    /// Pict number of the adaptive (`APal`-listed) picture being plotted.
    pub adaptive: u32,
    /// Pict number of the converter's pre-baked result.
    pub baked: u32,
}

/// The `BPal` table decoded from a Blorb's raw file bytes, or an empty vector
/// when the container has no `BPal` chunk.
///
/// Error-tolerant like the rest of the crate: a truncated container, or a chunk
/// whose length is not a multiple of 12, yields the rows that are fully present
/// rather than failing. `bytes` is the whole Blorb file (the same buffer handed
/// to [`crate::Blorb::parse`]).
pub fn palette_bakes(bytes: &[u8]) -> Vec<PaletteBake> {
    let Some(data) = top_level_chunk(bytes, b"BPal") else {
        return Vec::new();
    };
    data.as_chunks::<12>()
        .0
        .iter()
        .map(|r| PaletteBake {
            background: be_u32(r, 0),
            adaptive: be_u32(r, 4),
            baked: be_u32(r, 8),
        })
        .collect()
}

/// The data bytes of the first top-level chunk of type `ty`, or `None`. Mirrors
/// the chunk walk in [`crate::Blorb::parse`]: start after `FORM`+len+`IFRS`, and
/// stop at the first chunk whose declared length runs past the end.
fn top_level_chunk<'a>(bytes: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    if !crate::Blorb::is_blorb(bytes) {
        return None;
    }
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let ctype = &bytes[pos..pos + 4];
        let clen = be_u32(bytes, pos + 4) as usize;
        let data_start = pos + 8;
        let data_end = data_start.checked_add(clen)?;
        if data_end > bytes.len() {
            return None;
        }
        if ctype == ty {
            return Some(&bytes[data_start..data_end]);
        }
        pos = data_end + (clen & 1);
    }
    None
}

/// Big-endian u32 at `off`; 0 when out of range (callers are bounds-checked).
fn be_u32(b: &[u8], off: usize) -> u32 {
    match b.get(off..off + 4) {
        Some(s) => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal FORM/IFRS wrapper carrying one top-level chunk. No RIdx: the
    /// walk is independent of the resource index.
    fn container(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut inner = b"IFRS".to_vec();
        inner.extend_from_slice(ty);
        inner.extend_from_slice(&(data.len() as u32).to_be_bytes());
        inner.extend_from_slice(data);
        if data.len() % 2 == 1 {
            inner.push(0);
        }
        let mut file = b"FORM".to_vec();
        file.extend_from_slice(&(inner.len() as u32).to_be_bytes());
        file.extend_from_slice(&inner);
        file
    }

    fn rows(triples: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut v = Vec::new();
        for (a, b, c) in triples {
            for n in [a, b, c] {
                v.extend_from_slice(&n.to_be_bytes());
            }
        }
        v
    }

    #[test]
    fn decodes_triples_in_order() {
        let file = container(b"BPal", &rows(&[(1, 9, 1000), (1, 10, 1001), (2, 9, 1002)]));
        assert_eq!(
            palette_bakes(&file),
            vec![
                PaletteBake { background: 1, adaptive: 9, baked: 1000 },
                PaletteBake { background: 1, adaptive: 10, baked: 1001 },
                PaletteBake { background: 2, adaptive: 9, baked: 1002 },
            ]
        );
    }

    #[test]
    fn no_bpal_chunk_yields_no_rows() {
        assert!(palette_bakes(&container(b"APal", &[0, 0, 0, 9])).is_empty());
        assert!(palette_bakes(b"not a blorb").is_empty());
    }

    /// A trailing partial row is dropped rather than failing the decode — the
    /// crate never lets a malformed optional chunk sink the whole container.
    #[test]
    fn trailing_partial_row_is_dropped() {
        let mut data = rows(&[(1, 9, 1000)]);
        data.extend_from_slice(&[0, 0, 0, 2, 0, 0]); // 6 stray bytes
        let bakes = palette_bakes(&container(b"BPal", &data));
        assert_eq!(bakes.len(), 1);
        assert_eq!(bakes[0].baked, 1000);
    }
}
