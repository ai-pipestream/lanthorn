//! The **DOS sector order** a 5.25-inch Apple II floppy is dumped in, and the
//! one thing this module does about it: put the sectors back where ProDOS
//! expects them (SQ-0864).
//!
//! # What it is, and why it is a wrapper rather than a format
//!
//! An Apple II 5.25-inch disk is 35 tracks of 16 256-byte sectors — 143,360
//! bytes — and a raw dump of one stores those sectors in the order the *drive*
//! numbers them. ProDOS does not read sectors; it reads 512-byte **blocks**, two
//! sectors each, and it numbers them its own way. The two numberings disagree,
//! so a `.dsk` taken off an Apple II is a perfectly ordinary ProDOS volume whose
//! bytes are shuffled — the volume directory is not at offset 1024 and the sniff
//! in [`crate::prodos`] declines it, which is exactly what that module's header
//! said would happen until a reader arrived:
//!
//! > Image formats 0 and 2 are **not** re-ordered or decoded: a DOS-order or
//! > nibble image has its blocks somewhere else, so its volume directory does
//! > not validate and the sniff simply declines it.
//!
//! This is that re-ordering, and it is deliberately nothing else. It is the
//! third wrapper [`crate::prodos`] unwraps, beside a bare volume and a `2IMG`
//! header, and it leaves a ProDOS volume behind for the same reader to open.
//! **It is not a new disk format**: `blorb::medium` gains no row for it, because
//! what comes out the other side is ProDOS and answers to the ProDOS row.
//!
//! # The interleave
//!
//! Within one track, ProDOS block `b` (0..8) is DOS sectors `SECTOR_OF[2b]` and
//! `SECTOR_OF[2b + 1]`:
//!
//! ```text
//!   block  0   1   2   3   4   5   6   7
//!   first  0  13  11   9   7   5   3   1
//!   second 14  12  10   8   6   4   2  15
//! ```
//!
//! Sectors 0 and 15 stay put and the fourteen between them run backwards, which
//! is the shape every published table of this mapping has. It is stated here as
//! one flat array because that is how it is used.
//!
//! **Measured, not recalled.** The table is corroborated by the media itself,
//! which is the only authority that matters for a byte layout: applying it to
//! all nine 5.25-inch images in the corpus puts a valid ProDOS volume directory
//! at block 2 of every one of them — `SHOGUN.1`…`SHOGUN.5` and `ZORK0.1`…
//! `ZORK0.4` — each naming the segment files whose reassembly then verifies
//! against the story's own header checksum (see [`crate::infocom_packed`]). A
//! wrong table produces no volume directory at all, so there is nothing here for
//! a shared wrong assumption to hide behind.
//!
//! # Only this geometry
//!
//! DOS sector order is a 5.25-inch phenomenon and 5.25-inch DOS 3.3 media is 35
//! tracks, so [`prodos_order`] answers for exactly 143,360 bytes and declines
//! everything else. That is the same rule the rest of this crate applies to
//! spellings no medium in the corpus wears: a 40-track variant arrives with a
//! disk that is one, not before.

/// Tracks on a 5.25-inch DOS 3.3 floppy.
const TRACKS: usize = 35;

/// Sectors per track.
const SECTORS: usize = 16;

/// Bytes per sector. Two of them make one ProDOS block.
const SECTOR: usize = 256;

/// The one size a DOS-order 5.25-inch dump has: 35 × 16 × 256.
pub const DOS_ORDER_LEN: usize = TRACKS * SECTORS * SECTOR;

/// The DOS sector holding each successive half-block of a track, in ProDOS block
/// order. See the module header for the table this flattens.
const SECTOR_OF: [usize; SECTORS] = [0, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 15];

/// `raw` with its sectors put back into ProDOS block order, or `None` when it is
/// not a 5.25-inch DOS-order dump at all.
///
/// Says nothing about what the re-ordered bytes hold — that is
/// [`crate::prodos`]'s question, and a dump of a DOS 3.3 or Pascal disk comes
/// through here just as happily and is then declined by the volume sniff.
pub fn prodos_order(raw: &[u8]) -> Option<Vec<u8>> {
    if raw.len() != DOS_ORDER_LEN {
        return None;
    }
    let mut out = Vec::with_capacity(DOS_ORDER_LEN);
    for track in 0..TRACKS {
        let base = track * SECTORS * SECTOR;
        for sector in SECTOR_OF {
            let at = base + sector * SECTOR;
            out.extend_from_slice(&raw[at..at + SECTOR]);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mapping is a permutation of a track — every sector used exactly once,
    /// which is the one thing a transcription error would break.
    #[test]
    fn the_interleave_is_a_permutation_of_a_track() {
        let mut seen = SECTOR_OF;
        seen.sort_unstable();
        assert_eq!(seen, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    }

    /// Sector 0 opens the track and sector 15 closes it; the fourteen between
    /// them descend. Stated as a test because it is the whole shape of the
    /// table, and a table that lost it would still be a permutation.
    #[test]
    fn the_ends_stay_put_and_the_middle_runs_backwards() {
        assert_eq!(SECTOR_OF[0], 0);
        assert_eq!(SECTOR_OF[15], 15);
        assert!(SECTOR_OF[1..15].windows(2).all(|w| w[0] == w[1] + 1), "{SECTOR_OF:?}");
    }

    /// De-interleaving moves whole sectors and never a byte across a sector
    /// boundary: track `t`'s ProDOS block `b` is DOS sectors `2b` and `2b+1` of
    /// the table, byte for byte.
    #[test]
    fn every_sector_lands_where_the_table_says() {
        // Each sector filled with its own global index, so a misplacement is
        // visible in one byte.
        let raw: Vec<u8> = (0..DOS_ORDER_LEN).map(|i| (i / SECTOR) as u8).collect();
        let out = prodos_order(&raw).expect("the 5.25-inch geometry");
        assert_eq!(out.len(), DOS_ORDER_LEN);
        for track in 0..TRACKS {
            for (half, sector) in SECTOR_OF.iter().enumerate() {
                let at = (track * SECTORS + half) * SECTOR;
                let want = ((track * SECTORS + sector) % 256) as u8;
                assert_eq!(out[at], want, "track {track} half {half}");
                assert!(out[at..at + SECTOR].iter().all(|&b| b == want), "sector split");
            }
        }
    }

    /// Block 0 of every track is sectors 0 and 14 — the one case worth naming
    /// outright, because it is where a volume directory search begins.
    #[test]
    fn block_zero_of_a_track_is_sectors_zero_and_fourteen() {
        let mut raw = vec![0u8; DOS_ORDER_LEN];
        raw[0] = 0xA1; // track 0, sector 0
        raw[14 * SECTOR] = 0xA2; // track 0, sector 14
        let out = prodos_order(&raw).expect("the 5.25-inch geometry");
        assert_eq!(out[0], 0xA1);
        assert_eq!(out[SECTOR], 0xA2);
    }

    /// Any other size is not this medium, and is declined rather than
    /// re-ordered into nonsense. 800 KB is the size that matters — it is a
    /// multiple of a track and it is every `.2mg` in the corpus.
    #[test]
    fn only_the_five_and_a_quarter_inch_geometry_is_claimed() {
        assert_eq!(prodos_order(&[]), None);
        assert_eq!(prodos_order(&vec![0u8; DOS_ORDER_LEN - 1]), None);
        assert_eq!(prodos_order(&vec![0u8; DOS_ORDER_LEN + 1]), None);
        assert_eq!(prodos_order(&vec![0u8; 819_200]), None, "an 800 KB 3.5-inch volume");
        assert!(prodos_order(&vec![0u8; DOS_ORDER_LEN]).is_some());
    }
}
