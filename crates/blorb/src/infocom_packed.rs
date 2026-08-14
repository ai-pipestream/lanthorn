//! Reader for Infocom's **packed Apple volume** — the segmented `.D1`…`.D5`
//! container the Apple II releases of the graphical Version 6 games ship their
//! story in (SQ-0852).
//!
//! # What it is, and why it is not a filesystem
//!
//! *Arthur* and *Journey* do not put a story FILE on their ProDOS disks. What
//! they put there is a handful of opaque segments — `JOURNEY.D1`…`JOURNEY.D4`,
//! `ARTHUR.1/ARTHUR.D1`…`ARTHUR.5/ARTHUR.D5` — and the story is a **paging
//! image scattered across them by 512-byte block**. There is no filename to
//! find, no length, no contiguity: page 34 of the story can sit on the fourth
//! floppy while page 35 sits on the first. This module is the map that puts
//! them back in order.
//!
//! The container is a second container *inside* the filesystem volume, and it
//! is not the filesystem's business: the same index, byte for byte, addresses
//! the raw 5.25-inch pressings of *Shogun* and *Zork Zero*, which carry no
//! filesystem at all. So it lives on its own and takes a set of named byte
//! blobs rather than a [`crate::medium::Volume`].
//!
//! # The index
//!
//! Block 0 of the FIRST segment is an index, big-endian throughout:
//!
//! ```text
//!   0..2   unidentified (0x0083 Journey, 0x00fb Arthur, 0x00b9 Shogun,
//!                        0x0109 Zork Zero) — not a count, length or release
//!   2..4   number of segments, i.e. how many floppies the release was pressed on
//!   4..20  reserved, zero on every image in the corpus
//!  20..    one entry per segment, in segment order
//! ```
//!
//! and each entry is an 8-byte header followed by its runs:
//!
//! ```text
//!   0..2   a 16-bit checksum: the sum of the bytes of every block this segment
//!          contributes (see below — it is corroboration, not a key)
//!   2..4   page count, give or take (Journey's second entry reads one high and
//!          its first reads zero, so nothing depends on it)
//!   4..6   number of runs
//!   6..8   page count again on most entries, zero on others — likewise unused
//!   8..    `runs` records of six bytes: FIRST logical page, LAST logical page,
//!          FIRST physical block, all inclusive and all big-endian
//! ```
//!
//! A run says "story pages `first..=last` live on this segment starting at
//! block `physical`", so reading is a scatter-gather: walk every entry's runs,
//! fill a page table, then concatenate the pages in logical order. **The runs
//! tile the story's pages exactly** — every page from 0 to the last is named
//! once, which is the structural check this reader leans on hardest, because a
//! file that is not a packed volume has no reason to produce a gapless tiling.
//!
//! The index itself lives in the volume's own block space: on the ProDOS
//! releases it occupies block 0 of segment 1 and the story's page 0 is at block
//! 1, so the physical numbers are segment-file block numbers directly.
//!
//! # Why the checksum is required
//!
//! Nothing in a reassembled story says it was reassembled correctly — the pages
//! are opaque and a wrong map produces a plausible-looking file. The Z-machine
//! header carries the one oracle that settles it: `$1C` is the sum of every
//! byte from `$40` to the declared length (ZMSD §11.1.6), so this reader
//! ASSEMBLES and then VERIFIES, and hands back nothing that does not check out.
//! That is also what keeps a `.D1` full of something else from being misread
//! rather than refused.
//!
//! It is a strict test and it is meant to be: it is how *Arthur* release 63 was
//! proven readable, and it is what would catch a segment silently swapped for
//! another release's.
//!
//! # The entry checksum
//!
//! Entry `k`'s first word is the 16-bit sum of the bytes of the blocks segment
//! `k` contributes — verified on four of *Arthur*'s five segments and on
//! *Journey*'s. `ARTHUR.2/ARTHUR.D2` on the image in `stories/` does **not**
//! match it while still reassembling into a story whose header checksum is
//! exact, so that segment is a patched press and the word cannot be used to
//! identify a segment. It is documented here because it is real, and declined
//! as a key for the same reason.
//!
//! # Segments are paired by name, and then proven by content
//!
//! Recognising the FORMAT is by content, as everywhere in this crate: a segment
//! is the first segment because its block 0 parses as an index. Pairing the
//! REST is by name — `JOURNEY.D1`'s siblings are `JOURNEY.D2`…, `ARTHUR.D1`'s
//! are `ARTHUR.D2`… under their own directories — because the segments are
//! opaque page stores with nothing in them to sort by. The name only proposes;
//! the header checksum disposes.

use crate::adf::looks_like_story;

/// The block size the container pages in. ProDOS's, and the unit every physical
/// and logical number in the index counts.
const BLOCK: usize = 512;

/// How many segments an index may claim. Five is the most any release used
/// (*Arthur*, *Shogun*); the ceiling is a sanity bound on arbitrary bytes.
const MAX_SEGMENTS: usize = 8;

/// How many runs one segment's entry may hold. *Arthur*'s second segment uses
/// 35 and *Zork Zero*'s second uses 31; the bound keeps a nonsense count from
/// making this reader walk a whole disk image.
const MAX_RUNS: usize = 512;

/// One run: a contiguous range of story pages living at a contiguous range of
/// blocks on one segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    /// First story page this run supplies, inclusive.
    first_page: usize,
    /// Last story page this run supplies, inclusive.
    last_page: usize,
    /// Block on the segment holding `first_page`.
    block: usize,
}

/// The parsed index: one list of runs per segment, in segment order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Index {
    segments: Vec<Vec<Run>>,
}

/// Parse the index at the start of `first`, or `None` when these bytes are not
/// one.
///
/// Deliberately strict — see the module header. Everything a packed volume
/// states about itself has to be consistent before a single page is read.
fn parse_index(first: &[u8]) -> Option<Index> {
    if first.len() < BLOCK {
        return None;
    }
    let word = |o: usize| -> Option<usize> {
        let b = first.get(o..o + 2)?;
        Some(usize::from(u16::from_be_bytes([b[0], b[1]])))
    };
    // Bytes 4..20 are reserved and zero on every image in the corpus. It is the
    // cheapest thing that tells an index apart from arbitrary data.
    if first[4..20].iter().any(|&b| b != 0) {
        return None;
    }
    let count = word(2)?;
    if count == 0 || count > MAX_SEGMENTS {
        return None;
    }
    let mut at = 20;
    let mut segments = Vec::with_capacity(count);
    for _ in 0..count {
        let runs = word(at + 4)?;
        if runs == 0 || runs > MAX_RUNS {
            return None;
        }
        at += 8;
        let mut out = Vec::with_capacity(runs);
        for _ in 0..runs {
            let (first_page, last_page, block) = (word(at)?, word(at + 2)?, word(at + 4)?);
            if first_page > last_page {
                return None;
            }
            out.push(Run { first_page, last_page, block });
            at += 6;
        }
        segments.push(out);
    }
    Some(Index { segments })
}

/// The page table the index describes: for each story page, which segment and
/// which block on it.
///
/// `None` unless the runs tile `0..len` exactly — no gap anywhere below the
/// highest page named. Overlaps are allowed and the FIRST claim wins: both
/// *Journey* and *Arthur* genuinely name one page twice, and both reassemble to
/// a story whose header checksum is exact either way.
fn page_table(index: &Index) -> Option<Vec<(usize, usize)>> {
    let pages = index
        .segments
        .iter()
        .flat_map(|runs| runs.iter())
        .map(|r| r.last_page + 1)
        .max()
        .filter(|&n| n > 0)?;
    let mut table = vec![None; pages];
    for (segment, runs) in index.segments.iter().enumerate() {
        for run in runs {
            for (step, page) in (run.first_page..=run.last_page).enumerate() {
                table[page].get_or_insert((segment, run.block + step));
            }
        }
    }
    table.into_iter().collect()
}

/// The name of segment `n` (1-based) of a set whose first segment is stored as
/// `first`.
///
/// The releases spell it two ways and this covers both: `JOURNEY.D1` sits beside
/// `JOURNEY.D2` at the volume root, and `ARTHUR.1/ARTHUR.D1` beside
/// `ARTHUR.2/ARTHUR.D2`, so the match is on the trailing digit of the BASENAME
/// and the directory is not consulted at all.
fn sibling_of(first: &str, n: usize) -> Option<String> {
    let base = first.rsplit('/').next()?;
    let stem = base.strip_suffix('1')?;
    Some(format!("{stem}{n}"))
}

/// Reassemble the story a packed Apple volume holds, out of the segment files
/// `files` — or `None` when these files are not one, or do not hold all of it.
///
/// The returned name is the FIRST segment's, the one carrying the index: it is
/// where the volume begins, and it is a name the caller was shown and can ask
/// for again.
///
/// A set missing a segment is `None` rather than a truncated story. That is not
/// hypothetical — `stories/Journey.2mg` declares five segments and carries four,
/// so 92 of its 552 pages are simply not on the image.
pub fn story(files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    files.iter().find_map(|(name, bytes)| assemble(name, bytes, files))
}

/// Reassemble, taking `first` as the segment holding the index.
fn assemble(name: &str, first: &[u8], files: &[(String, Vec<u8>)]) -> Option<(String, Vec<u8>)> {
    let index = parse_index(first)?;
    let table = page_table(&index)?;

    // Segment 1 is the file the index came off; the rest are its namesakes.
    let mut segments: Vec<&[u8]> = Vec::with_capacity(index.segments.len());
    segments.push(first);
    for n in 2..=index.segments.len() {
        let want = sibling_of(name, n)?;
        let found = files
            .iter()
            .find(|(other, _)| other.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(&want)))?;
        segments.push(&found.1);
    }

    let mut story = Vec::with_capacity(table.len() * BLOCK);
    for &(segment, block) in &table {
        let at = block.checked_mul(BLOCK)?;
        story.extend_from_slice(segments[segment].get(at..at + BLOCK)?);
    }
    verified(story).map(|bytes| (name.to_string(), bytes))
}

/// `story` truncated to its declared length, if it really is a story and its
/// header checksum agrees with the bytes that were reassembled.
///
/// The whole point of the module's strictness lands here; see the header for
/// why nothing weaker will do.
fn verified(mut story: Vec<u8>) -> Option<Vec<u8>> {
    if !looks_like_story(&story) {
        return None;
    }
    let word = |o: usize| usize::from(u16::from_be_bytes([story[o], story[o + 1]]));
    // ZMSD §11.1.6: the length field counts in the version's packed-address
    // scale, which is 8 for every Version 6 story — and Version 6 is the only
    // version this container was ever used for.
    let scale = match story[0] {
        3 => 2,
        4 | 5 => 4,
        6..=8 => 8,
        _ => return None,
    };
    let (length, checksum) = (word(0x1a) * scale, word(0x1c));
    if length < 64 || length > story.len() {
        return None;
    }
    let sum = story[64..length].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
    if checksum != 0 && usize::from(sum) != checksum {
        return None;
    }
    story.truncate(length);
    Some(story)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A story whose header checksum is correct for its own bytes, so the
    /// reader's one hard test can pass without a real game.
    fn story_bytes(len: usize) -> Vec<u8> {
        let mut b = vec![0u8; len];
        b[0] = 6;
        let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
        word(0x04, 0x0400); // high memory
        word(0x08, 0x0300); // dictionary
        word(0x0a, 0x0100); // objects
        word(0x0c, 0x0200); // globals
        word(0x0e, 0x0280); // static memory base
        word(0x1a, (len / 8) as u16);
        b[0x12..0x18].copy_from_slice(b"890616");
        for (i, slot) in b.iter_mut().enumerate().skip(64) {
            *slot = (i % 251) as u8;
        }
        let sum = b[64..len].iter().fold(0u16, |a, &c| a.wrapping_add(u16::from(c)));
        b[0x1c..0x1e].copy_from_slice(&sum.to_be_bytes());
        b
    }

    /// Build a two-segment packed volume holding `story`, with the pages
    /// deliberately out of order and split across both segments so that a
    /// reader which merely concatenates cannot pass.
    fn packed(story: &[u8]) -> Vec<(String, Vec<u8>)> {
        let pages = story.len().div_ceil(BLOCK);
        let half = pages / 2;
        // Segment 1 holds the index in block 0 and the SECOND half of the story
        // from block 1; segment 2 holds the first half, reversed.
        let mut d1 = vec![0u8; BLOCK * (1 + (pages - half))];
        for (i, page) in (half..pages).enumerate() {
            let at = BLOCK * (1 + i);
            let src = page * BLOCK;
            let end = (src + BLOCK).min(story.len());
            d1[at..at + (end - src)].copy_from_slice(&story[src..end]);
        }
        let mut d2 = vec![0u8; BLOCK * half];
        for page in 0..half {
            let at = BLOCK * (half - 1 - page);
            d2[at..at + BLOCK].copy_from_slice(&story[page * BLOCK..(page + 1) * BLOCK]);
        }
        let mut index: Vec<u8> = Vec::new();
        index.extend_from_slice(&0x0083u16.to_be_bytes()); // the unidentified word
        index.extend_from_slice(&2u16.to_be_bytes()); // two segments
        index.resize(20, 0);
        // Segment 1: one run, pages half..pages at block 1.
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&((pages - half) as u16).to_be_bytes());
        index.extend_from_slice(&1u16.to_be_bytes());
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&((pages - 1) as u16).to_be_bytes());
        index.extend_from_slice(&1u16.to_be_bytes());
        // Segment 2: one run per page, descending on the disk.
        index.extend_from_slice(&0u16.to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&(half as u16).to_be_bytes());
        index.extend_from_slice(&0u16.to_be_bytes());
        for page in 0..half {
            index.extend_from_slice(&(page as u16).to_be_bytes());
            index.extend_from_slice(&(page as u16).to_be_bytes());
            index.extend_from_slice(&((half - 1 - page) as u16).to_be_bytes());
        }
        assert!(index.len() <= BLOCK, "the sample index must fit block 0");
        d1[..index.len()].copy_from_slice(&index);
        vec![("GAME.D1".into(), d1), ("GAME.D2".into(), d2)]
    }

    #[test]
    fn a_scattered_story_is_put_back_in_logical_page_order() {
        let want = story_bytes(BLOCK * 9);
        let files = packed(&want);
        let (name, got) = story(&files).expect("the packed volume reassembles");
        assert_eq!(name, "GAME.D1", "the name is the segment carrying the index");
        assert_eq!(got, want);
    }

    /// The container's own proof of correctness. Swapping two pages leaves every
    /// structural check intact — the tiling is still gapless, every block still
    /// exists — and only the header checksum notices.
    #[test]
    fn a_map_that_puts_one_page_in_the_wrong_place_is_refused() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        let d1 = &mut files[0].1;
        // Entry 2's first two runs name pages 0 and 1; trade their blocks.
        let a = 20 + 8 + 6 + 8 + 4;
        let b = a + 6;
        d1.swap(a, b);
        d1.swap(a + 1, b + 1);
        assert_eq!(story(&files), None, "a mis-assembled story must not be handed out");
    }

    #[test]
    fn a_set_missing_a_segment_is_refused_rather_than_truncated() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        files.truncate(1);
        assert_eq!(story(&files), None, "Journey.2mg's exact defect, in two lines");
    }

    #[test]
    fn ordinary_files_are_not_packed_volumes() {
        assert_eq!(story(&[("STORY.DAT".into(), story_bytes(BLOCK * 4))]), None);
        assert_eq!(story(&[("junk".into(), vec![0u8; BLOCK * 4])]), None);
        assert_eq!(story(&[("short".into(), vec![7u8; 12])]), None);
        assert_eq!(story(&[]), None);
    }

    /// The reserved words are what a random file trips over first.
    #[test]
    fn a_first_block_with_dirt_in_its_reserved_words_is_not_an_index() {
        let want = story_bytes(BLOCK * 9);
        let mut files = packed(&want);
        files[0].1[7] = 1;
        assert_eq!(story(&files), None);
    }

    #[test]
    fn segments_are_paired_by_the_basename_under_any_directory() {
        assert_eq!(sibling_of("JOURNEY.D1", 3).as_deref(), Some("JOURNEY.D3"));
        assert_eq!(sibling_of("ARTHUR.1/ARTHUR.D1", 5).as_deref(), Some("ARTHUR.D5"));
        assert_eq!(sibling_of("STORY.DAT", 2), None);
    }

    // ── Real media ───────────────────────────────────────────────────────────

    /// The user's `stories/` directory, gitignored — every test over it skips
    /// vacuously, and CI has none of it at all.
    fn stories_dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    /// Every file on a ProDOS image in `stories/`, or `None` when it is absent.
    fn volume_files(name: &str) -> Option<Vec<(String, Vec<u8>)>> {
        let path = stories_dir().join(name);
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        };
        let fs = crate::prodos::ProDos::mount(raw).expect("a ProDOS image in the corpus mounts");
        Some(fs.files().iter().filter_map(|e| fs.read(e).map(|b| (e.path(), b))).collect())
    }

    /// **The headline.** *Arthur*'s Apple press keeps its story in five segments
    /// and no file on the volume is a story; this is the whole container,
    /// end to end, proven by the game's own header checksum.
    #[test]
    fn real_arthur_reassembles_out_of_five_segments() {
        let Some(files) = volume_files("Arthur Quest 4 Excalibur.2mg") else { return };
        assert!(
            files.iter().all(|(_, b)| !looks_like_story(b)),
            "no FILE on this volume is a story — that is what makes it a packed volume"
        );
        let (name, bytes) = story(&files).expect("the packed volume reassembles");
        assert_eq!(name, "ARTHUR.1/ARTHUR.D1", "named for the segment carrying the index");
        assert_eq!(bytes.len(), 271_304, "the story's own declared length");
        assert_eq!(bytes[0], 6, "Version 6");
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 63, "release 63");
        assert_eq!(&bytes[0x12..0x18], b"890622", "serial 890622");
        // `story` already required this; asserted again because it is the one
        // fact that says the page map was right rather than merely plausible.
        let sum = bytes[64..].iter().fold(0u16, |a, &b| a.wrapping_add(u16::from(b)));
        assert_eq!(sum, u16::from_be_bytes([bytes[0x1c], bytes[0x1d]]), "ZMSD §11.1.6 checksum");
    }

    /// **`Journey.2mg` is an incomplete pressing, and is refused as one.**
    ///
    /// Its index declares five segments; the volume carries `JOURNEY.D1` …
    /// `JOURNEY.D4` and nothing else, so 92 of the story's 552 pages are simply
    /// not on the image. A reader that concatenated what it had would produce
    /// 460 pages of plausible-looking Z-code; this one produces nothing.
    #[test]
    fn real_journey_is_refused_because_its_fifth_segment_is_absent() {
        let Some(files) = volume_files("Journey.2mg") else { return };
        let (name, first) = files
            .iter()
            .find(|(n, b)| n.ends_with("D1") && parse_index(b).is_some())
            .expect("JOURNEY.D1 carries an index");
        let index = parse_index(first).expect("…and it parses");
        assert_eq!(index.segments.len(), 5, "the index declares five segments");
        assert_eq!(
            (2..=5).filter(|&n| {
                let want = sibling_of(name, n).unwrap();
                files.iter().any(|(o, _)| o.rsplit('/').next() == Some(want.as_str()))
            })
            .count(),
            3,
            "and only D2, D3 and D4 are on the volume"
        );
        assert_eq!(story(&files), None, "so no story is handed out at all");
    }

    /// Nothing else in the corpus is mistaken for a packed volume — including
    /// the six *Lost Treasures* volumes, which are ProDOS disks full of ordinary
    /// story files, and the standalone *Beyond Zork*.
    #[test]
    fn no_other_prodos_image_in_the_corpus_holds_a_packed_volume() {
        let Ok(dir) = std::fs::read_dir(stories_dir()) else {
            eprintln!("SKIP: no stories directory");
            return;
        };
        let mut ran = 0;
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            if !name.to_ascii_lowercase().ends_with(".2mg") {
                continue;
            }
            let Some(files) = volume_files(&name) else { continue };
            ran += 1;
            let packed = story(&files);
            let expected = name == "Arthur Quest 4 Excalibur.2mg";
            assert_eq!(packed.is_some(), expected, "{name}: packed volume found = {expected}");
        }
        if ran == 0 {
            eprintln!("SKIP: no ProDOS media present");
        }
    }
}
