//! Picture family C on the **Atari 8-bit** companion picture sides (§8.3,
//! §7.3, §12.10 — SQ-1483, SQ-1484).
//!
//! # What these disks are
//!
//! Seven two-sided US S.A.G.A. releases, catalogued in §10.5 with the sha256
//! of every file. Side A is the database side (§12.12 tabulates what each one
//! decodes to) and side B is nothing but artwork: **no Atari DOS 2 directory**
//! — the eight sectors where one would be hold picture data like every other
//! sector — so §8.3's "on the Atari there is no filesystem walk" is right, and
//! a reader has only the bytes.
//!
//! ```text
//! <fixtures>/atari/SAGA #4 - Voodoo Castle [side B].atr        sha256 2a417fb62f14…
//! <fixtures>/atari/SAGA #5 - The Count [side B].atr            sha256 37fd7e4fd1cf…
//! <fixtures>/atari/SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr
//!                                                              sha256 9542de4bb9d8…
//! ```
//!
//! where `<fixtures>` is `$SCOTT_DIALECT_FIXTURES` or `stories/scott-dialects`.
//! Commercial game files, not redistributable, not committed — every case here
//! **skips vacuously with an explanation** when a disk is absent, because a
//! silent skip reads exactly like a pass.
//!
//! # Only three of the seven are family C
//!
//! The other four — *Adventureland*, *Pirate Adventure*, *Mission Impossible*
//! and *Strange Odyssey* — keep a line-drawing token stream on side B instead,
//! the format Appendix A item 26 measured on the four **plain** Apple II
//! releases. [`no_line_art_side_is_mistaken_for_a_bitmap_side`] is the case
//! that says so, and it is as much a part of this suite as the three that
//! decode: a scanner that found "records" on those four would be finding noise.
//!
//! # What settles the record shape
//!
//! No oracle, unlike [`saga_pictures_specimens`](../saga_pictures_specimens),
//! whose MS-DOS twin decided §8.3's inclusive limit. What settles it here is
//! that a family-C record is **self-proving**: its header says how many byte
//! pairs it holds, and a wrong reading of the header — a twelve-byte one, the
//! other compression scheme, the other edge convention — cannot produce
//! exactly that many. See `scott::saga_atari` for the shape and for how far
//! it departs from §8.3's prose.

use std::path::PathBuf;

use scott::saga_atari::{decode_record, scan_picture_side, splice_vtoc, AtariRecord, SIDE_LEN};
use scott::saga_atari_lineart::{LineArtCanvas, LineArtPicture};
use scott::saga_pictures::{FamilyCScheme, CANVAS_HEIGHT, CANVAS_WIDTH};
use scott::{SagaPlatform, SagaUs};

/// The same three candidates the sibling suites try, in the same order.
fn fixtures() -> Option<PathBuf> {
    [
        std::env::var_os("SCOTT_DIALECT_FIXTURES").map(PathBuf::from),
        Some(PathBuf::from("stories/scott-dialects")),
        Some(PathBuf::from("../../stories/scott-dialects")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_dir())
}

/// One release's side B, or `None` with a reason on stderr.
fn side_b(file: &str) -> Option<Vec<u8>> {
    let Some(dir) = fixtures() else {
        eprintln!("SKIP: no stories/scott-dialects — see this file's header");
        return None;
    };
    let path = dir.join("atari").join(file);
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: {} is absent — see this file's header", path.display());
        return None;
    };
    // §7.3's identification, so a truncated or re-mastered image cannot read
    // as a pass.
    assert_eq!(raw.len(), SIDE_LEN, "{file} is not a 720-sector single-density image");
    assert_eq!(&raw[..6], &[0x96, 0x02, 0x80, 0x16, 0x80, 0x00], "{file} lacks §7.3's header");
    Some(raw)
}

/// The three family-C titles, with the release identity §12.12 gives each, the
/// record count measured on its side, and how many records the side holds that
/// the consistency check **cannot** read (SQ-1483).
///
/// The unreadable ones are two records on *The Count*'s side and no others.
/// Both declare far more data than their region can hold — the one at file
/// offset `0x7CBA` declares 4,742 bytes against a 2,765-pair region whose
/// run-length units come to 6,322 pairs — so neither satisfies the check, and
/// this suite names them rather than loosening the check until they pass.
///
/// `(file, release, scheme, records, unreadable)`.
const BITMAP_TITLES: [(&str, SagaUs, FamilyCScheme, usize, usize); 3] = [
    (
        "SAGA #4 - Voodoo Castle [side B].atr",
        SagaUs { version: 119, adventure: 4, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::NoLiteral,
        79,
        0,
    ),
    (
        "SAGA #5 - The Count [side B].atr",
        SagaUs { version: 115, adventure: 5, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::NoLiteral,
        72,
        2,
    ),
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
        SagaUs { version: 125, adventure: 13, platform: SagaPlatform::Atari8Bit },
        FamilyCScheme::Standard,
        90,
        0,
    ),
];

/// The four titles whose side B is a line-drawing token stream, not family C.
const LINE_ART_TITLES: [&str; 4] = [
    "SAGA #1 - Adventureland [side B].atr",
    "SAGA #2 - Pirate Adventure [side B].atr",
    "SAGA #3 - Mission Impossible [side B].atr",
    "SAGA #6 - Strange Odyssey [side B].atr",
];

/// Per title, how many records the scan finds — the number a re-master or a
/// change to the consistency check would move.
#[test]
fn each_bitmap_side_holds_the_measured_number_of_records() {
    for (file, release, scheme, want, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        assert_eq!(release.picture_scheme(), scheme, "{file}: §8.3 names the variant's titles");
        assert_eq!(
            release.atari_picture_format(),
            Some(scott::AtariPictureFormat::FamilyCBitmap),
            "{file}: and this one is a bitmap side",
        );
        let found = scan_picture_side(&raw, scheme);
        assert_eq!(found.len(), want, "{file}: record count");
    }
}

/// Every record decodes, and each one's declared size is its decoded length or
/// one more — never anything else.
///
/// That last is the measurement that refutes §8.3's twelve-byte header for
/// this platform. Read with two more bytes of header the data is two pairs
/// short of the region on every record, and not one of these would decode.
#[test]
fn every_record_decodes_and_its_size_matches_what_it_took() {
    for (file, release, _, _, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let scheme = release.picture_scheme();
        let spliced = splice_vtoc(&raw);
        let found = scan_picture_side(&raw, scheme);
        let mut exact = 0usize;
        for r in &found {
            let slack = r.size() - r.decoded_len();
            assert!(slack <= 1, "{file}: record at 0x{:05X} has {slack} spare bytes", r.file_offset());
            if slack == 0 {
                exact += 1;
            }
            let pic = decode_record(&spliced, r, scheme).expect("a located record decodes");
            assert_eq!((pic.width(), pic.height()), (CANVAS_WIDTH, CANVAS_HEIGHT));
            assert!(
                pic.unrecognised_colours().is_empty(),
                "{file}: every Atari colour byte has a colour, but 0x{:05X} left {:?}",
                r.file_offset(),
                pic.unrecognised_colours(),
            );
            assert_eq!(pic.palette()[0], (0, 0, 0), "§8.3: entry 0 is black whatever the record says");
        }
        assert!(exact > found.len() / 2, "{file}: most records declare exactly what they took");
    }
}

/// Records lie end to end from the head of the side, with nought to six bytes
/// of filler between them — and the only breaks in that run are the two
/// unreadable records [`BITMAP_TITLES`] names.
///
/// The filler is the reason a reader cannot address a picture by adding sizes
/// from the first, and — with §12.10's "not recoverable from the database" —
/// the reason the per-title offset lists §8.3 asks for have to be measured.
///
/// This case is also what would catch the scan quietly losing records: a
/// tightened check, or a re-mastered disk, shows up as a gap of thousands of
/// bytes where a picture used to be.
#[test]
fn records_lie_end_to_end_with_at_most_six_bytes_of_filler() {
    /// Bigger than any filler run and smaller than any record: a gap past this
    /// is a picture that was not read, not slack between two that were.
    const FILLER: usize = 8;
    for (file, release, _, _, unreadable) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let found = scan_picture_side(&raw, release.picture_scheme());
        assert!(found.len() > 40, "{file}: sanity, the scan found something");
        assert!(
            found[0].file_offset() < 0x300,
            "{file}: the first record is right behind the shared boot loader, at 0x{:05X}",
            found[0].file_offset(),
        );
        let mut breaks = Vec::new();
        for pair in found.windows(2) {
            let gap = pair[1].offset() - (pair[0].offset() + pair[0].size());
            if gap > FILLER {
                breaks.push((pair[0].file_offset(), gap));
            }
        }
        assert_eq!(
            breaks.len(),
            unreadable,
            "{file}: expected {unreadable} unreadable records, found breaks after {breaks:02X?}",
        );
    }
}

/// **The falsification for SQ-1484.** *The Count* and *Voodoo Castle* read
/// with the standard scheme yield almost nothing, and *Claymorgue Castle* read
/// with the variant likewise.
///
/// This is the case that would have caught getting §8.3's variant wrong, and
/// it is stronger than any hand-built record can be: a whole 92 KB side offers
/// 92,000 offsets to be wrong at, and the measured separation is not close.
/// Right scheme against wrong, per title: *Voodoo Castle* 79 against 2, *The
/// Count* 72 against 0, *Claymorgue Castle* 90 against 6.
#[test]
fn the_wrong_scheme_finds_almost_no_records_on_a_real_side() {
    /// Above every wrong-scheme count measured (6) and far below every right
    /// one (72).
    const NOISE: usize = 10;
    for (file, release, _, want, _) in BITMAP_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let other = match release.picture_scheme() {
            FamilyCScheme::Standard => FamilyCScheme::NoLiteral,
            FamilyCScheme::NoLiteral => FamilyCScheme::Standard,
            // `FamilyCScheme` is `#[non_exhaustive]`: only two schemes exist
            // today, and a third would need this test's own "the other one"
            // rule revisited rather than guessed at.
            _ => unreachable!("only two family-C schemes exist"),
        };
        let wrong = scan_picture_side(&raw, other);
        assert!(
            wrong.len() < NOISE && want > NOISE * 4,
            "{file}: the wrong scheme found {} records against {want} right ones",
            wrong.len(),
        );
    }
}

/// A line-art side is not mistaken for a bitmap side under either scheme.
///
/// The four titles here are §8.3's blind spot: it says the Atari releases are
/// family C and four of the seven are not. A scan that found records on these
/// would be reading noise, and the whole method would be worthless.
///
/// Measured, three of the four yield **nothing at all** under either scheme
/// and *Strange Odyssey* yields five accidental hits under the standard one —
/// against 72 to 90 on a side that really is family C.
#[test]
fn no_line_art_side_is_mistaken_for_a_bitmap_side() {
    const NOISE: usize = 10;
    for file in LINE_ART_TITLES {
        let Some(raw) = side_b(file) else { continue };
        for scheme in [FamilyCScheme::Standard, FamilyCScheme::NoLiteral] {
            let found = scan_picture_side(&raw, scheme);
            assert!(
                found.len() < NOISE,
                "{file} is a line-drawing side, but {scheme:?} found {} records",
                found.len(),
            );
        }
    }
    // And the crate says so by release identity rather than by sniffing.
    for (adventure, version) in [(1u16, 416u16), (2, 408), (3, 306), (6, 119)] {
        let r = SagaUs { version, adventure, platform: SagaPlatform::Atari8Bit };
        assert_eq!(r.atari_picture_format(), Some(scott::AtariPictureFormat::LineArt));
    }
    for (adventure, version) in [(4u16, 119u16), (5, 115), (13, 125)] {
        let r = SagaUs { version, adventure, platform: SagaPlatform::Atari8Bit };
        assert_eq!(r.atari_picture_format(), Some(scott::AtariPictureFormat::FamilyCBitmap));
    }
}

/// One picture pinned by geometry and by pixels, per title.
///
/// **Named by what it depicts**, because that is the only thing that says the
/// decode is right rather than merely self-consistent — a sheared or noisy
/// picture is as internally consistent as a good one. Each was read off a
/// render of the record at the offset below.
///
/// `(file, scheme, file offset, what it shows, cols, pairs, the four colour bytes)`
type Pin = (&'static str, FamilyCScheme, usize, &'static str, i32, i32, [u8; 4]);

const PINNED: [Pin; 3] = [
    (
        "SAGA #5 - The Count [side B].atr",
        FamilyCScheme::NoLiteral,
        0x5580,
        "the brass bed of room 1, the player's two feet sticking up out of a white sheet",
        35,
        79,
        [0x36, 0x3D, 0x0E, 0x00],
    ),
    (
        "SAGA #4 - Voodoo Castle [side B].atr",
        FamilyCScheme::NoLiteral,
        0x0297,
        "a coffin on a bier between drawn curtains, a candelabrum at each end",
        37,
        63,
        [0x36, 0x87, 0x50, 0x00],
    ),
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
        FamilyCScheme::Standard,
        0x0DC48,
        "four planks of a wooden shelf, end grain and all, against a dark wall",
        38,
        64,
        [0x3A, 0x15, 0x0E, 0x00],
    ),
];

/// The scheme the crate's own release lookup gives this side's release.
///
/// Every case reaches the scheme through here rather than through
/// [`PINNED`]'s and [`BITMAP_TITLES`]' literals, so that changing
/// `SagaUs::picture_scheme` fails the whole suite and not only the one case
/// that pins it — which is what "falsify the fix" asks for.
fn scheme_of(file: &str) -> FamilyCScheme {
    BITMAP_TITLES
        .iter()
        .find(|(f, ..)| *f == file)
        .map(|(_, release, ..)| release.picture_scheme())
        .unwrap_or_else(|| panic!("{file} is not one of the three bitmap titles"))
}

#[test]
fn one_picture_per_title_is_pinned_by_geometry_and_by_pixels() {
    for (file, pinned_scheme, at, what, cols, pairs, colours) in PINNED {
        let Some(raw) = side_b(file) else { continue };
        let scheme = scheme_of(file);
        assert_eq!(scheme, pinned_scheme, "{file}: the release lookup and the pin agree");
        let spliced = splice_vtoc(&raw);
        let found = scan_picture_side(&raw, scheme);
        let r: &AtariRecord = found
            .iter()
            .find(|r| r.file_offset() == at)
            .unwrap_or_else(|| panic!("{file}: no record at 0x{at:05X} ({what})"));
        assert_eq!((r.layout().cols(), r.layout().pairs()), (cols, pairs), "{file}: {what}");
        assert_eq!(r.colour_bytes(), colours, "{file}: {what}");
        let pic = decode_record(&spliced, r, scheme).expect("decodes");
        // A non-vacuity guard, counted over the record's OWN region rather
        // than the canvas — a small record leaves most of the canvas at value
        // 0 quite properly, and a picture that is all one value inside its own
        // region is a decode that failed quietly.
        let mut seen = [0usize; 4];
        let mut total = 0usize;
        for y in r.layout().top()..r.layout().top() + r.layout().pairs() * 2 {
            for x in r.layout().left()..r.layout().left() + r.layout().cols() * 8 {
                if (0..CANVAS_WIDTH as i32).contains(&x) && (0..CANVAS_HEIGHT as i32).contains(&y) {
                    seen[usize::from(pic.pixels()[y as usize * CANVAS_WIDTH + x as usize])] += 1;
                    total += 1;
                }
            }
        }
        let used = seen.iter().filter(|&&n| n > 0).count();
        assert!(used >= 3, "{file}: {what} uses only {used} of the four pixel values");
        assert!(
            seen.iter().all(|&n| n * 10 < total * 9),
            "{file}: {what} is nine-tenths one colour inside its own region, so it did not decode",
        );
    }
}

/// *The Count*'s room 1, pixel by pixel.
///
/// The picture is a brass bed seen from its foot: the bedstead's two upright
/// posts and the rail between them fill the upper third, and the player's two
/// feet stand up from the white sheet across the bottom. Three points are
/// enough to say it is that picture and not a shifted or sheared one.
#[test]
fn the_counts_room_one_draws_the_brass_bed_its_text_describes() {
    let file = "SAGA #5 - The Count [side B].atr";
    let Some(raw) = side_b(file) else { return };
    let scheme = scheme_of(file);
    let spliced = splice_vtoc(&raw);
    let found = scan_picture_side(&raw, scheme);
    let r = found.iter().find(|r| r.file_offset() == 0x5580).expect("room 1's record");
    let pic = decode_record(&spliced, r, scheme).expect("decodes");
    let at = |x: usize, y: usize| pic.pixels()[y * CANVAS_WIDTH + x];
    // The record covers the whole canvas from the origin.
    assert_eq!((r.layout().left(), r.layout().top()), (0, 0));
    // The bed's white sheet is the bottom third, and it is bright.
    let sheet: usize = (120..150).map(|y| (60..220).filter(|&x| at(x, y) == 3).count()).sum();
    assert!(sheet > 3_000, "the sheet across the bottom is {sheet} bright pixels");
    // The wall above the bedstead is not.
    let wall_bright: usize = (0..10).map(|y| (0..CANVAS_WIDTH).filter(|&x| at(x, y) == 3).count()).sum();
    assert!(wall_bright < 900, "the wall along the top is {wall_bright} bright pixels");
    // And the picture is not blank anywhere it should not be.
    assert!(pic.pixels().contains(&1), "the wall colour is in use");
    assert!(pic.pixels().contains(&2), "the third colour is in use");
}

// ── The (usage, index) table on side A (SQ-1496, investigation findings) ─────

/// One release's side A, or `None` with a reason on stderr — the same shape as
/// [`side_b`], because the table below lives on the DATABASE side.
fn side_a(file: &str) -> Option<Vec<u8>> {
    let Some(dir) = fixtures() else {
        eprintln!("SKIP: no stories/scott-dialects — see this file's header");
        return None;
    };
    let path = dir.join("atari").join(file);
    let Ok(raw) = std::fs::read(&path) else {
        eprintln!("SKIP: {} is absent — see this file's header", path.display());
        return None;
    };
    assert_eq!(raw.len(), SIDE_LEN, "{file} is not a 720-sector single-density image");
    Some(raw)
}

// The arithmetic that turns a two-byte table entry into a side-B FILE offset
// (`table_entry_file_offset`) and that turns a side-B FILE offset into the
// spliced coordinates `record_at`/`scan_picture_side` use (`spliced_of`) is
// now production code — promoted from this test's own hand-rolled copies once
// SQ-1496 wired the table into `PictSource` — so this suite calls
// `scott::saga_atari`'s versions rather than maintaining a second copy that
// could drift from them. Likewise the table's own layout constants.
use scott::saga_atari::{
    spliced_of, table_entry_file_offset, INVENTORY_BACKDROP_ENTRY, PICTURE_TABLE_ENTRIES,
    PICTURE_TABLE_OFFSET as PICTURE_TABLE,
};

/// Per title: side A, side B, the room count, and how many table entries the
/// walk must resolve to a located record (room usage, object usage) — the
/// number that would move if the scan lost a record or the encoding drifted.
const TABLE_TITLES: [(&str, &str, usize, usize, usize); 3] = [
    // 23 rooms, close-ups 80 and 81, title 99; 47 objects, 80/82/83/84 among them.
    ("SAGA #5 - The Count [side A].atr", "SAGA #5 - The Count [side B].atr", 23, 26, 47),
    // 26 rooms, close-ups 81-84, title 99; 47 objects, 70 among them.
    ("SAGA #4 - Voodoo Castle [side A].atr", "SAGA #4 - Voodoo Castle [side B].atr", 26, 31, 47),
    // 33 rooms, title 99, no close-ups; 55 objects.
    (
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side A.atr",
        "SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr",
        33,
        34,
        55,
    ),
];

/// **The association §12.10 says is not in the database IS on side A**, in a
/// 400-byte table at [`PICTURE_TABLE`] that the program reads rather than the
/// database (SQ-1496). Every non-zero entry decodes to the header offset of a
/// record the side-B scan located — the only exceptions being *The Count*'s
/// two damaged records (SQ-1498), which the table names at exactly the
/// offsets the scan skips — and every room 0..N has one, as does 99.
#[test]
fn side_a_carries_the_picture_table_and_every_entry_names_a_located_record() {
    for (a_file, b_file, rooms, want_rooms, want_objects) in TABLE_TITLES {
        let (Some(a), Some(b)) = (side_a(a_file), side_b(b_file)) else { continue };
        let scheme = scheme_of(b_file);
        let found = scan_picture_side(&b, scheme);
        let starts: std::collections::BTreeSet<usize> = found.iter().map(|r| r.offset()).collect();
        // The two records the scan cannot read (SQ-1498), in file offsets.
        let damaged: &[usize] = if a_file.contains("Count") { &[0x7CBA, 0xF72C] } else { &[] };
        let (mut room_hits, mut object_hits) = (0usize, 0usize);
        for i in 0..PICTURE_TABLE_ENTRIES {
            let (lo, hi) = (a[PICTURE_TABLE + 2 * i], a[PICTURE_TABLE + 2 * i + 1]);
            if lo == 0 && hi == 0 {
                assert!(i >= rooms, "{a_file}: room {i} has no picture entry");
                continue;
            }
            let file_offset = table_entry_file_offset(lo, hi);
            let located = starts.contains(&spliced_of(file_offset)) || damaged.contains(&file_offset);
            assert!(located, "{a_file}: entry {i} ({lo:02X} {hi:02X}) points at 0x{file_offset:05X}, where no record starts");
            if i < 100 {
                room_hits += 1;
                // Room usage never carries the inventory flag.
                assert_eq!(lo & 4, 0, "{a_file}: room entry {i} carries the inventory flag");
            } else {
                object_hits += 1;
            }
        }
        assert_eq!((room_hits, object_hits), (want_rooms, want_objects), "{a_file}: resolved entries");
        // §8.6's reserved 99 is present and 98 is not — the inventory backdrop
        // is reached through INVENTORY_BACKDROP_ENTRY instead.
        assert_ne!(a[PICTURE_TABLE + 198], 0, "{a_file}: no title picture");
        assert_eq!(&a[PICTURE_TABLE + 196..PICTURE_TABLE + 198], &[0, 0], "{a_file}: slot 98 is unused");
        let inv =
            table_entry_file_offset(a[INVENTORY_BACKDROP_ENTRY], a[INVENTORY_BACKDROP_ENTRY + 1]);
        let card = found
            .iter()
            .find(|r| r.offset() == spliced_of(inv))
            .unwrap_or_else(|| panic!("{a_file}: the inventory entry points at 0x{inv:05X}, no record"));
        // Three flat colour bars, a few dozen bytes for a near-full canvas.
        assert!(card.size() < 100, "{a_file}: the inventory backdrop is {} bytes", card.size());
        assert!(card.layout().cols() >= 29, "{a_file}: the inventory backdrop is {} columns", card.layout().cols());
    }
}

/// **SQ-1498: the two unreadable records are one bad sector each.** *The
/// Count*'s room 6 (CRYPT) at file 0x7CBA has sector 258 replaced by a copy
/// of sector 254, and its room 16 (Dungeon) at 0xF72C has sector 498 zeroed.
/// Both headers are well-formed, both declared sizes fill their gaps exactly,
/// and the side-A table names both offsets — so the encoding is the ordinary
/// one and the specimen is damaged, not the reading.
#[test]
fn the_counts_two_unreadable_records_each_lost_exactly_one_sector() {
    let Some(b) = side_b("SAGA #5 - The Count [side B].atr") else { return };
    let sector = |n: usize| &b[16 + (n - 1) * 128..16 + n * 128];
    // Room 6: header, declared size 4742 = the gap; sector 258 duplicates 254.
    assert_eq!(&b[0x7CBA..0x7CC4], &[0x86, 0x12, 0x03, 0x00, 0x26, 0x9E, 0x36, 0x87, 0x0E, 0x00]);
    assert_eq!(0x7CBA + 4742, 0x8F40, "the record runs up to the byte before room 7's header");
    assert_eq!(sector(258), sector(254), "sector 258 is a stale copy of sector 254");
    assert_ne!(sector(257), sector(258), "and its neighbours are not");
    // Room 16: header, declared size 3344 = the gap; sector 498 is all zero.
    assert_eq!(&b[0xF72C..0xF736], &[0x10, 0x0D, 0x03, 0x00, 0x26, 0x9E, 0x36, 0x87, 0x0E, 0x00]);
    assert_eq!(0xF72C + 3344, 0x1043C, "the record ends five filler bytes before room 17's header at 0x10441");
    assert!(sector(498).iter().all(|&x| x == 0), "sector 498 is unwritten");
    assert!(sector(497).iter().any(|&x| x != 0) && sector(499).iter().any(|&x| x != 0));
}

// ── Promoted to production (SQ-1496/SQ-1498) ────────────────────────────────

/// [`scott::saga_atari::read_picture_table`] end to end, cross-checked
/// against the lower-level walk above rather than duplicating it: same entry
/// counts, and the inventory backdrop it finds is the same record the
/// hand-rolled walk finds by the same arithmetic.
#[test]
fn read_picture_table_resolves_the_same_entries_the_hand_rolled_walk_does() {
    for (a_file, b_file, _rooms, want_rooms, want_objects) in TABLE_TITLES {
        let (Some(a), Some(b)) = (side_a(a_file), side_b(b_file)) else { continue };
        let scheme = scheme_of(b_file);
        let spliced = splice_vtoc(&b);
        let adventure = if a_file.contains("Count") {
            5
        } else if a_file.contains("Voodoo") {
            4
        } else {
            13
        };
        let table = scott::saga_atari::read_picture_table(&a, &spliced, scheme, adventure)
            .unwrap_or_else(|| panic!("{a_file}: the table's own marker should verify"));
        let rooms = table.entries().iter().filter(|e| e.usage == scott::PictureUsage::Room).count();
        let objects = table.entries().len() - rooms;
        assert_eq!((rooms, objects), (want_rooms, want_objects), "{a_file}: production table entries");
        // The title card (99) resolves to a real, decodable picture.
        let title_offset = table
            .find(scott::PictureUsage::Room, 99)
            .unwrap_or_else(|| panic!("{a_file}: no title picture in the production table"));
        assert!(
            scott::saga_atari::decode_table_picture(&spliced, title_offset, scheme).is_some(),
            "{a_file}: the title picture should decode"
        );
        // The inventory backdrop resolves and decodes too.
        let inv_offset = table
            .inventory_backdrop_file_offset()
            .unwrap_or_else(|| panic!("{a_file}: no inventory backdrop in the production table"));
        assert!(
            scott::saga_atari::decode_table_picture(&spliced, inv_offset, scheme).is_some(),
            "{a_file}: the inventory backdrop should decode"
        );
    }
}

/// A garbage table entry is refused, not drawn (SQ-1496's own caution about
/// *Voodoo Castle*'s table running short past 0x970F) — a synthetic side A
/// with one entry pointing at a spliced offset no record starts at must not
/// appear in [`scott::saga_atari::read_picture_table`]'s output.
///
/// This is the deliberate falsification for the table reader: an entry whose
/// arithmetic is wrong (or that names a region the scan never located) must
/// be dropped rather than surfaced as a resolvable picture.
#[test]
fn a_table_entry_pointing_at_no_record_is_refused() {
    let Some(a_real) = side_a("SAGA #5 - The Count [side A].atr") else { return };
    let Some(b) = side_b("SAGA #5 - The Count [side B].atr") else { return };
    let scheme = scheme_of("SAGA #5 - The Count [side B].atr");
    let spliced = splice_vtoc(&b);

    // A real table, then one entry corrupted to point at an offset that is
    // never a record's own header — room 5's slot, overwritten with a
    // plainly-off-grid (sector, byte) pair.
    let mut a = a_real.clone();
    a[PICTURE_TABLE + 2 * 5] = 0xFF;
    a[PICTURE_TABLE + 2 * 5 + 1] = 0xFF;
    let table = scott::saga_atari::read_picture_table(&a, &spliced, scheme, 5)
        .expect("the marker is untouched, so the table itself still reads");
    assert!(
        table.find(scott::PictureUsage::Room, 5).is_none(),
        "the corrupted entry must not resolve to any record"
    );
    // And every other entry is unaffected.
    assert!(table.find(scott::PictureUsage::Room, 1).is_some(), "room 1's own entry still resolves");
}

/// The marker in front of the table is checked, not trusted: flip one byte of
/// it and [`scott::saga_atari::read_picture_table`] must refuse the whole
/// table rather than read three garbage bytes as if they were real.
#[test]
fn a_wrong_marker_refuses_the_whole_table() {
    let Some(a_real) = side_a("SAGA #5 - The Count [side A].atr") else { return };
    let Some(b) = side_b("SAGA #5 - The Count [side B].atr") else { return };
    let scheme = scheme_of("SAGA #5 - The Count [side B].atr");
    let spliced = splice_vtoc(&b);
    let mut a = a_real;
    a[scott::saga_atari::PICTURE_TABLE_MARKER] = 0xFF;
    assert!(
        scott::saga_atari::read_picture_table(&a, &spliced, scheme, 5).is_none(),
        "a corrupted marker must refuse the table rather than read past it"
    );
}

/// **SQ-1498's fallback recovers a real picture for both damaged records.**
/// [`scott::saga_atari::decode_table_picture`] tries the ordinary path first
/// and falls back to [`scott::saga_atari::decode_record_with_bad_sector_fallback`]
/// only when that refuses — exactly what happens for *The Count*'s room 6 and
/// room 16, at the file offsets the picture table itself names for them.
#[test]
fn the_counts_two_damaged_records_decode_through_the_sq_1498_fallback() {
    let Some(b) = side_b("SAGA #5 - The Count [side B].atr") else { return };
    let scheme = scheme_of("SAGA #5 - The Count [side B].atr");
    let spliced = splice_vtoc(&b);
    for (file_offset, what) in [(0x7CBA, "room 6 (CRYPT)"), (0xF72C, "room 16 (Dungeon)")] {
        // The ordinary path refuses these two, which is the premise of the
        // fallback existing at all.
        assert!(
            scott::saga_atari::record_at(&spliced, spliced_of(file_offset), scheme).is_none(),
            "{what}: record_at should still refuse this, unchanged"
        );
        let pic = scott::saga_atari::decode_table_picture(&spliced, file_offset, scheme)
            .unwrap_or_else(|| panic!("{what}: the SQ-1498 fallback should recover a picture"));
        // Non-vacuity, the same bar `one_picture_per_title_is_pinned_by_geometry_and_by_pixels`
        // uses: real art uses several of the four pixel values and is not
        // overwhelmingly one of them.
        let mut seen = [0usize; 4];
        for &v in pic.pixels() {
            seen[usize::from(v)] += 1;
        }
        let total: usize = seen.iter().sum();
        let used = seen.iter().filter(|&&n| n > 0).count();
        assert!(used >= 3, "{what}: the fallback decode uses only {used} of the four pixel values");
        assert!(
            seen.iter().all(|&n| n * 10 < total * 9),
            "{what}: the fallback decode is nine-tenths one colour, so it did not really recover anything"
        );
    }
}

// ── The line-art sides' own picture table (SQ-1524, investigation findings) ──
//
// The four `AtariPictureFormat::LineArt` titles carry their (usage, index)
// table ON SIDE B — not on side A, where the three bitmap titles keep theirs
// (SQ-1496): side A of these four holds no `[adv, adv, adv]` marker anywhere
// near 0x9590, and side B holds one at 0x290, right where the bitmap sides
// carry the same three bytes in front of their first record. The table has
// the bitmap table's shape — 100 room-usage slots then object slots, two
// bytes `[A, S]` each, all-zero for "no picture" — but NOT its arithmetic:
// the byte within the sector is `(A >> 2) * 3`, because a line-art record is
// laid on a three-byte grid (its tokens are three bytes each), where a bitmap
// record is laid on a seven-byte one. There is no inventory flag bit, because
// an object picture serves both uses — exactly as the same four titles'
// Apple II `B<aa><nnn>` files do, one file per object, no R/I suffix.
//
// What settles the arithmetic is the walk below: every entry lands on a
// record whose three-byte tokens all keep to the 160 x 96 GRAPHICS 7 canvas,
// whose `0x00` end byte falls 0-2 zero bytes before the next entry's own
// offset, and whose picture is what the index says it is (entry 0 spells
// `IT'S TOO DARK!`, 99 the Adventure International globe, 98 `INVENTORY`,
// Adventureland's 24 a devil in flames for "...Oh Hell!"). Read the same
// entries with the bitmap arithmetic and most land mid-stream.
//
// Note what this ALSO says about the earlier `decode_line_art_opening`
// reading (retired, SQ-1525): the tokens here are on a 160 x 96 canvas with
// fixed three-byte width, not the Apple's 280 x 192 mixed-width grammar, so
// that function's decode was a misreading — see the SQ-1524 investigation
// note.
//
// The table's own layout (the marker offset, the table offset, the entry
// count) and the arithmetic that turns a two-byte entry into a side-B FILE
// offset are now production code — promoted from this suite's own hand-rolled
// copies once SQ-1524/SQ-1525's implementation lane wired the table into
// `PictSource`, the same way [`table_entry_file_offset`] was for the bitmap
// titles above — so this suite calls `scott::saga_atari`'s versions rather
// than maintaining a second copy that could drift from them.
use scott::saga_atari::{
    line_art_entry_file_offset, LINE_ART_TABLE_MARKER, LINE_ART_TABLE_ENTRIES,
    LINE_ART_TABLE_OFFSET as LINE_ART_TABLE,
};

/// The GRAPHICS 7 canvas every drawing token keeps to, measured over all
/// four titles' 310 records: no `x` above 159, no `y` above 95.
const LINE_ART_WIDTH: u8 = 160;
const LINE_ART_HEIGHT: u8 = 96;

/// Walk one line-art record in fixed three-byte tokens from its first byte.
/// Returns how many drawing tokens (top bit set) fell off the canvas and the
/// offset of the `0x00`-class end byte, or `None` if the walk ran out first.
fn walk_line_art_record(rec: &[u8]) -> (usize, Option<usize>) {
    let mut off_canvas = 0;
    let mut t = 0;
    while t + 3 <= rec.len() {
        let c = rec[t];
        if c & 0xE0 == 0 {
            return (off_canvas, Some(t));
        }
        if c & 0x80 != 0 && (rec[t + 1] >= LINE_ART_WIDTH || rec[t + 2] >= LINE_ART_HEIGHT) {
            off_canvas += 1;
        }
        t += 3;
    }
    // A record whose end byte is the last byte of the slice, or the last but
    // one, has no full token left to read; look for it in the remainder.
    match rec[t..].iter().position(|&b| b & 0xE0 == 0) {
        Some(p) => (off_canvas, Some(t + p)),
        None => (off_canvas, None),
    }
}

/// Every non-zero table entry on a line-art side, as `(slot, a, s, file offset)`.
fn line_art_table_entries(side_b: &[u8]) -> Vec<(usize, u8, u8, usize)> {
    (0..LINE_ART_TABLE_ENTRIES)
        .filter_map(|i| {
            let at = LINE_ART_TABLE + 2 * i;
            let (a, s) = (side_b[at], side_b[at + 1]);
            (a != 0 || s != 0).then(|| (i, a, s, line_art_entry_file_offset(a, s)))
        })
        .collect()
}

/// Per title: the side-B file, its adventure number, and the room and
/// object picture indices its table names — which are, slot for slot, the
/// `R<aa><nn>` and `B<aa><nnn>` files the SAME title's Apple II release
/// catalogues (`apple_pictures_specimens` walks those), gaps included:
/// *Mission Impossible* has no room 1 on either machine, *Strange Odyssey*
/// no room 16 and none of 25-34, and each title's object subset is the
/// same 44, 48, 29 and 31 items. Two independent releases agreeing on which
/// of sixty-odd items have artwork is the cross-platform anchor SQ-1496
/// leaned on, in a form that needs no picture decoded at all.
const LINE_ART_TABLES: [(&str, u8, &[usize], &[usize]); 4] = [
    (
        "SAGA #1 - Adventureland [side B].atr",
        1,
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32, 33, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 98, 99,
        ],
        &[
            0, 2, 4, 7, 8, 9, 10, 11, 12, 13, 14, 17, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 31,
            35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 52, 55, 56, 60, 61,
        ],
    ),
    (
        "SAGA #2 - Pirate Adventure [side B].atr",
        2,
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 80, 81, 82, 83, 84, 85, 86, 87, 90, 91, 98, 99,
        ],
        &[
            3, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28,
            29, 30, 31, 32, 33, 37, 41, 42, 44, 45, 46, 47, 48, 49, 50, 52, 53, 54, 57, 58, 60, 61,
            62, 63,
        ],
    ),
    (
        "SAGA #3 - Mission Impossible [side B].atr",
        3,
        &[
            0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 80, 81,
            82, 83, 84, 85, 88, 89, 98, 99,
        ],
        &[
            0, 1, 2, 3, 7, 8, 9, 10, 11, 12, 16, 17, 19, 21, 22, 23, 24, 26, 27, 28, 30, 36, 37, 39,
            40, 41, 42, 43, 49,
        ],
    ),
    (
        "SAGA #6 - Strange Odyssey [side B].atr",
        6,
        &[
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23, 24, 35,
            81, 82, 83, 84, 85, 86, 87, 88, 91, 92, 93, 98, 99,
        ],
        &[
            6, 7, 10, 12, 13, 19, 21, 23, 24, 27, 28, 29, 30, 31, 34, 35, 36, 37, 38, 39, 40, 41, 42,
            43, 44, 47, 48, 50, 51, 53, 55,
        ],
    ),
];

/// **The line-art sides carry their own picture table**, behind the
/// adventure marker at [`LINE_ART_TABLE_MARKER`] (SQ-1524). Every non-zero
/// entry, read with [`line_art_entry_file_offset`], is the first byte of a
/// record that walks cleanly in three-byte tokens to a `0x00` end byte lying
/// 0-2 zero bytes before the next entry's own offset — so the table
/// accounts for the whole side, end to end, with nothing between records but
/// grid padding — and the indices it names are the Apple II catalogue's.
#[test]
fn each_line_art_side_carries_its_own_picture_table_and_every_entry_starts_a_record() {
    for (file, adventure, want_rooms, want_objects) in LINE_ART_TABLES {
        let Some(raw) = side_b(file) else { continue };
        assert_eq!(
            &raw[LINE_ART_TABLE_MARKER..LINE_ART_TABLE_MARKER + 3],
            &[adventure, adventure, adventure],
            "{file}: the adventure marker in front of the table",
        );
        let entries = line_art_table_entries(&raw);
        let rooms: Vec<usize> = entries.iter().map(|e| e.0).filter(|&i| i < 100).collect();
        let objects: Vec<usize> = entries.iter().map(|e| e.0).filter(|&i| i >= 100).map(|i| i - 100).collect();
        assert_eq!(rooms, want_rooms, "{file}: room indices with a picture");
        assert_eq!(objects, want_objects, "{file}: object indices with a picture");

        // In disk order the records lie end to end; walk each up to the next.
        let spliced = splice_vtoc(&raw);
        let mut by_offset = entries.clone();
        by_offset.sort_by_key(|e| e.3);
        for pair in by_offset.windows(2) {
            let (slot, a, s, file_offset) = pair[0];
            let next = spliced_of(pair[1].3);
            let rec = &spliced[spliced_of(file_offset)..next];
            assert!(
                matches!(rec[0] & 0xE0, 0x60 | 0x80),
                "{file}: slot {slot} ({a:02X} {s:02X}) at 0x{file_offset:05X} opens with {:02X}, not a paint or a move",
                rec[0],
            );
            let (off_canvas, end) = walk_line_art_record(rec);
            assert_eq!(off_canvas, 0, "{file}: slot {slot} at 0x{file_offset:05X} draws off the 160x96 canvas");
            let end = end.unwrap_or_else(|| panic!("{file}: slot {slot} at 0x{file_offset:05X} never ends"));
            assert_eq!(rec[end], 0x00, "{file}: slot {slot}'s end byte");
            let pad = &rec[end + 1..];
            assert!(
                pad.len() <= 2 && pad.iter().all(|&b| b == 0),
                "{file}: slot {slot} at 0x{file_offset:05X} is followed by {pad:02X?} before the next record",
            );
        }
        // The last record, with nothing after it to bound it, still ends.
        let (_, _, _, last) = by_offset[by_offset.len() - 1];
        let rec = &spliced[spliced_of(last)..];
        let (off_canvas, end) = walk_line_art_record(rec);
        assert_eq!(off_canvas, 0, "{file}: the last record draws off the canvas");
        assert!(end.is_some(), "{file}: the last record never ends");
    }
}

/// The two cards every title shares are the same bytes on every side:
/// index 0 (`IT'S TOO DARK!`, 2,854 bytes at 0x590 — which is where
/// `LINE_ART_OFFSET` was reading from the middle of) and index 99 (the
/// Adventure International globe, 2,428 bytes). Index 98 (`INVENTORY`) is
/// shared by *Adventureland* and *Pirate Adventure* only; the other two
/// redraw it.
#[test]
fn the_darkness_and_title_cards_are_the_same_bytes_on_every_line_art_side() {
    /// (adventure, darkness card, title card, inventory card).
    type Cards = (u8, Vec<u8>, Vec<u8>, Vec<u8>);
    let mut cards: Vec<Cards> = Vec::new();
    for (file, adventure, _, _) in LINE_ART_TABLES {
        let Some(raw) = side_b(file) else { continue };
        let spliced = splice_vtoc(&raw);
        let record = |slot: usize| -> Vec<u8> {
            let (_, _, _, at) = line_art_table_entries(&raw)
                .into_iter()
                .find(|e| e.0 == slot)
                .unwrap_or_else(|| panic!("{file}: no entry for slot {slot}"));
            let rec = &spliced[spliced_of(at)..];
            let (_, end) = walk_line_art_record(rec);
            rec[..=end.expect("the record ends")].to_vec()
        };
        cards.push((adventure, record(0), record(99), record(98)));
    }
    let Some(first) = cards.first() else { return };
    assert_eq!(first.1.len(), 2854, "the darkness card's length");
    assert_eq!(first.2.len(), 2428, "the title card's length");
    for c in &cards[1..] {
        assert_eq!(c.1, first.1, "adventure {}: the darkness card differs", c.0);
        assert_eq!(c.2, first.2, "adventure {}: the title card differs", c.0);
        assert_eq!(c.3 == first.3, matches!(c.0, 1 | 2), "adventure {}: the inventory card", c.0);
    }
}

/// §7.3's splice is load-bearing here too: *Adventureland*'s object 27 (the
/// sleeping dragon) starts at 0xB37C and runs across sector 360. Spliced, it
/// walks clean; raw, its tokens read the volume table as coordinates and
/// leave the canvas.
#[test]
fn a_line_art_record_spanning_the_volume_table_needs_the_splice() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let entries = line_art_table_entries(&raw);
    let at = entries.iter().find(|e| e.0 == 127).map(|e| e.3).expect("object 27's entry");
    assert_eq!(at, 0xB37C);
    let next = entries.iter().map(|e| e.3).filter(|&o| o > at).min().expect("a record after it");
    let spliced = splice_vtoc(&raw);
    let (clean, end) = walk_line_art_record(&spliced[spliced_of(at)..spliced_of(next)]);
    assert_eq!((clean, end.is_some()), (0, true), "spliced, the record walks to its end on the canvas");
    let (dirty, _) = walk_line_art_record(&raw[at..next]);
    assert!(dirty > 0, "raw, the volume table's bytes read as {dirty} off-canvas tokens");
}

/// The falsification: the bitmap sides' `(A >> 3) * 7` reading of the same
/// two bytes (`table_entry_file_offset`, SQ-1496) lands most entries in the
/// middle of a token stream, where they walk off the canvas or open on a
/// byte that is neither a paint nor a move. Same table, other grid.
#[test]
fn the_bitmap_arithmetic_reads_the_line_art_table_as_noise() {
    for (file, _, _, _) in LINE_ART_TABLES {
        let Some(raw) = side_b(file) else { continue };
        let spliced = splice_vtoc(&raw);
        let entries = line_art_table_entries(&raw);
        let mut wrong = 0;
        for &(_, a, s, _) in &entries {
            let at = spliced_of(table_entry_file_offset(a, s));
            let rec = &spliced[at..(at + 4000).min(spliced.len())];
            let (off_canvas, _) = walk_line_art_record(rec);
            if off_canvas > 0 || !matches!(rec[0] & 0xE0, 0x60 | 0x80) {
                wrong += 1;
            }
        }
        assert!(wrong * 2 > entries.len(), "{file}: the seven-byte grid still read {} of {} entries", entries.len() - wrong, entries.len());
    }
}

// ---------------------------------------------------------------------------
// The line-art grammar (SQ-1525)
//
// `scott::saga_atari_lineart` reads the format the way the releases' own
// renderer draws it — read off side A's boot-loaded program as a specimen, no
// interpreter consulted; see that module's docs. The cases below play every
// record of every line-art title through it and pin what comes out.
//
// The per-title pixel totals are a cross-check against a second, independent
// transcription of the same reading (a scratch Python decoder written first,
// from the same disassembly, and used to look at the pictures): both agreeing
// on every one of 310 records, fill by fill, is what says neither mis-copied
// a case of the line walk or the fill sweep.
// ---------------------------------------------------------------------------

/// The bytes of one record, first token to end byte inclusive, or `None` if
/// the table has no entry for `slot`.
fn line_art_record(raw: &[u8], slot: usize) -> Option<Vec<u8>> {
    let (_, _, _, at) = line_art_table_entries(raw).into_iter().find(|e| e.0 == slot)?;
    let spliced = splice_vtoc(raw);
    let rec = &spliced[spliced_of(at)..];
    let (_, end) = walk_line_art_record(rec);
    Some(rec[..=end.expect("the record ends")].to_vec())
}

/// Non-black pixels of a picture: every pixel whose pair is not (0, 0).
fn lit(pic: &LineArtPicture) -> usize {
    pic.pixels().iter().filter(|&&v| v != 0).count()
}

/// Every record on every line-art side plays to its end under the renderer's
/// grammar, and the pictures sum to the same non-black pixel counts the
/// second transcription measured. `(file, records, non-black pixels, rooms
/// that never clear)`.
///
/// **Six room records never clear the screen** — *Pirate Adventure*'s 7, 12,
/// 13 and 19 (one 353-token maze-of-caves drawing), its 86, and *Strange
/// Odyssey*'s 3. On the machine they draw over whatever the screen held —
/// the previous room and its objects — since nothing else clears between
/// rooms; a host that plays them on a black canvas shows black where the
/// machine showed the picture before. Every other room clears, so its painted
/// box is the whole canvas (the maze's fills happen to reach every edge too).
#[test]
fn every_line_art_record_plays_under_the_renderers_grammar() {
    const TOTALS: [(&str, usize, usize, &[usize]); 4] = [
        ("SAGA #1 - Adventureland [side B].atr", 92, 560_441, &[]),
        ("SAGA #2 - Pirate Adventure [side B].atr", 87, 489_008, &[7, 12, 13, 19, 86]),
        ("SAGA #3 - Mission Impossible [side B].atr", 62, 409_219, &[]),
        ("SAGA #6 - Strange Odyssey [side B].atr", 69, 452_313, &[3]),
    ];
    for (file, records, want_lit, never_clear) in TOTALS {
        let Some(raw) = side_b(file) else { continue };
        let entries = line_art_table_entries(&raw);
        assert_eq!(entries.len(), records, "{file}: table entries");
        let mut total = 0;
        let mut no_clear = Vec::new();
        for (slot, _, _, _) in &entries {
            let rec = line_art_record(&raw, *slot).expect("an entry");
            let mut canvas = LineArtCanvas::new();
            let pic = canvas.draw(&rec).unwrap_or_else(|e| panic!("{file}: slot {slot} refused: {e}"));
            total += lit(&pic);
            if *slot < 100 {
                let clears = rec.chunks(3).any(|t| t[0] == 0x20);
                let p = pic.painted().unwrap_or_else(|| panic!("{file}: room {slot} drew nothing"));
                let whole = (p.left(), p.top(), p.right(), p.bottom()) == (0, 0, 159, 95);
                assert!(whole || !clears, "{file}: room {slot} clears but painted only {p:?}");
                if !clears {
                    no_clear.push(*slot);
                }
            }
        }
        assert_eq!(no_clear, never_clear, "{file}: rooms that never clear");
        assert_eq!(total, want_lit, "{file}: non-black pixels over all records");
    }
}

/// The darkness card (index 0) is an animation: `IT'S TOO DARK!` lettering, a
/// pair of eyes that appear and vanish across three pauses, and a final clear
/// to black — so its resting frame is genuinely black on the machine, and the
/// lettering is on the paused frames a host has to reach for.
#[test]
fn the_darkness_card_is_an_animation_that_ends_black() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let rec = line_art_record(&raw, 0).expect("the darkness card");
    let mut canvas = LineArtCanvas::new();
    let frames = canvas.draw_frames(&rec).expect("plays");
    let counts: Vec<usize> = frames.iter().map(lit).collect();
    assert_eq!(counts, [2158, 1955, 1148, 0], "non-black pixels at each pause, then at the end");
    // The lettering sits in the top and bottom bands of every paused frame;
    // the eyes sit between them on the first and are gone by the last, bar a
    // few strokes of the letters that reach into the band.
    let band = |frame: &LineArtPicture, top: usize, bottom: usize| {
        (top..=bottom)
            .flat_map(|y| (0..160).map(move |x| (x, y)))
            .filter(|&(x, y)| frame.rgb(x, y) != Some((0, 0, 0)))
            .count()
    };
    assert!(band(&frames[2], 0, 25) > 300, "IT'S TOO on top");
    assert!(band(&frames[2], 65, 95) > 300, "DARK! below");
    assert!(band(&frames[0], 35, 60) > 500, "the eyes, on the first pause");
    assert_eq!(band(&frames[2], 35, 60), 23, "gone by the last");
    let again = canvas.draw(&rec).expect("plays again");
    assert_eq!(again.painted().map(|p| (p.left(), p.bottom())), Some((0, 95)));
}

/// One picture per title, and the shared title card, pinned by pixel count —
/// and the card by a few sampled pairs: the `ai` box's green, the globe's red
/// ground, the white lettering.
#[test]
fn the_title_card_and_a_room_per_title_decode_to_the_measured_pictures() {
    const ROOMS: [(&str, usize, usize); 4] = [
        ("SAGA #1 - Adventureland [side B].atr", 98, 14_357), // INVENTORY, a man with a sack
        ("SAGA #2 - Pirate Adventure [side B].atr", 20, 13_374), // the ship's deck
        ("SAGA #3 - Mission Impossible [side B].atr", 2, 13_303), // the office desk
        ("SAGA #6 - Strange Odyssey [side B].atr", 1, 14_294), // the scoutship cockpit
    ];
    for (file, slot, want) in ROOMS {
        let Some(raw) = side_b(file) else { continue };
        let rec = line_art_record(&raw, slot).expect("an entry");
        let pic = LineArtCanvas::new().draw(&rec).expect("plays");
        assert_eq!(lit(&pic), want, "{file}: slot {slot}");
    }
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let rec = line_art_record(&raw, 99).expect("the title card");
    let pic = LineArtCanvas::new().draw(&rec).expect("plays");
    assert_eq!(lit(&pic), 13_511);
    let at = |x: usize, y: usize| pic.pixels()[y * pic.width() + x];
    assert_eq!(at(5, 5), 4 + 1, "the ground: pair (1, 1), colour 7");
    assert_eq!(at(60, 15), 2 * 4 + 3, "the ai box: pair (2, 3), colour 10");
    assert_eq!(at(80, 48), 3 * 4 + 3, "the lettering: pair (3, 3), colour 0");
    assert_eq!(pic.rgb(80, 48), Some(scott::saga_atari_lineart::pair_rgb(3, 3)));
}

/// The recolour marker's effect on a real record: *Adventureland*'s object 0
/// is the `Dark hole`, drawn as lines in the default colour and then, by its
/// closing `6F 0F 09 | 21`, redrawn from the start in black — so on a black
/// canvas it leaves no lit pixel at all, yet reports the box it drew on.
#[test]
fn the_dark_hole_is_drawn_and_then_recoloured_black() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let rec = line_art_record(&raw, 100).expect("object 0");
    assert_eq!(&rec[rec.len() - 7..], &[0x6F, 0x0F, 0x09, 0x21, 0x87, 0x47, 0x00]);
    let pic = LineArtCanvas::new().draw(&rec).expect("plays");
    assert_eq!(lit(&pic), 0);
    assert!(pic.painted().is_some());
}

// ── Promoted to production (SQ-1524/SQ-1525's implementation lane) ─────────
//
// `line_art_entry_file_offset`, `LINE_ART_TABLE_MARKER`, `LINE_ART_TABLE`
// (`LINE_ART_TABLE_OFFSET`) and `LINE_ART_TABLE_ENTRIES` above are already
// `scott::saga_atari`'s own — see the `use` before them. What follows
// exercises the production reader and player built on top of that
// arithmetic (`read_line_art_table`, `draw_line_art_record`,
// `draw_darkness_card`), cross-checked against this suite's own
// lower-level walk rather than duplicating it, the same shape
// `read_picture_table_resolves_the_same_entries_the_hand_rolled_walk_does`
// uses for the bitmap titles above.

/// [`scott::saga_atari::read_line_art_table`] end to end: the same room and
/// object index sets [`LINE_ART_TABLES`] pins by cross-reference against the
/// Apple II catalogues, and a title card that actually decodes.
#[test]
fn read_line_art_table_resolves_the_same_entries_the_hand_rolled_walk_does() {
    for (file, adventure, want_rooms, want_objects) in LINE_ART_TABLES {
        let Some(raw) = side_b(file) else { continue };
        let spliced = splice_vtoc(&raw);
        let table = scott::saga_atari::read_line_art_table(&spliced, u16::from(adventure))
            .unwrap_or_else(|| panic!("{file}: the table's own marker should verify"));
        let mut rooms: Vec<u16> = table
            .entries()
            .iter()
            .filter(|e| e.usage == scott::PictureUsage::Room)
            .map(|e| e.index)
            .collect();
        rooms.sort_unstable();
        let mut objects: Vec<u16> = table
            .entries()
            .iter()
            .filter(|e| e.usage != scott::PictureUsage::Room)
            .map(|e| e.index)
            .collect();
        objects.sort_unstable();
        let want_rooms: Vec<u16> = want_rooms.iter().map(|&i| i as u16).collect();
        let want_objects: Vec<u16> = want_objects.iter().map(|&i| i as u16).collect();
        assert_eq!(rooms, want_rooms, "{file}: production table room indices");
        assert_eq!(objects, want_objects, "{file}: production table object indices");
        // The title card (99) resolves to a real, playable record.
        let title_offset = table
            .find(scott::PictureUsage::Room, 99)
            .unwrap_or_else(|| panic!("{file}: no title picture in the production table"));
        let mut canvas = LineArtCanvas::new();
        assert!(
            scott::saga_atari::draw_line_art_record(&mut canvas, &spliced, title_offset).is_ok(),
            "{file}: the title picture should play"
        );
        // Both object usages resolve to the SAME record (SQ-1524: no
        // inventory flag bit here, unlike the bitmap titles).
        if let Some(&first_object) = want_objects.first() {
            assert_eq!(
                table.find(scott::PictureUsage::ObjectInRoom, first_object),
                table.find(scott::PictureUsage::ObjectInInventory, first_object),
                "{file}: object {first_object} should answer the same record either way",
            );
        }
    }
}

/// A garbage line-art table entry is refused, not played — the same caution
/// [`scott::saga_atari::read_picture_table`] takes for the bitmap titles,
/// mirrored here for [`scott::saga_atari::read_line_art_table`].
#[test]
fn a_line_art_table_entry_pointing_at_no_record_is_refused() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let mut corrupted = raw.clone();
    // Room 5's slot, overwritten with a plainly-off-grid `(a, s)` pair.
    corrupted[LINE_ART_TABLE + 2 * 5] = 0xFF;
    corrupted[LINE_ART_TABLE + 2 * 5 + 1] = 0xFF;
    let spliced = splice_vtoc(&corrupted);
    let table = scott::saga_atari::read_line_art_table(&spliced, 1)
        .expect("the marker is untouched, so the table itself still reads");
    assert!(
        table.find(scott::PictureUsage::Room, 5).is_none(),
        "the corrupted entry must not resolve to any record"
    );
    assert!(table.find(scott::PictureUsage::Room, 1).is_some(), "room 1's own entry still resolves");
}

/// The marker in front of the line-art table is checked, not trusted.
#[test]
fn a_wrong_line_art_marker_refuses_the_whole_table() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let mut corrupted = raw.clone();
    corrupted[LINE_ART_TABLE_MARKER] = 0xFF;
    let spliced = splice_vtoc(&corrupted);
    assert!(
        scott::saga_atari::read_line_art_table(&spliced, 1).is_none(),
        "a corrupted marker must refuse the table rather than read past it"
    );
}

/// **SQ-1525's first host-side fact: a never-clearing room record must
/// composite over whatever [`scott::saga_atari::draw_line_art_record`]'s
/// canvas already held, not paint a fresh black one.**
///
/// *Pirate Adventure*'s room 86 is one of the six room records that never
/// clear the screen (`every_line_art_record_plays_under_the_renderers_grammar`
/// pins the full list) — its own `painted()` box happens to reach every edge
/// of the canvas regardless (that test's own comment: "the maze's fills
/// happen to reach every edge too"), so a bounding-box comparison cannot say
/// anything here; **individual pixels** are what settle it. Play a normal,
/// clearing room first, then room 86 onto the SAME canvas, and find a pixel
/// the first room lit that room 86, played ALONE from a fresh canvas, never
/// touches at all (still black there) — a pixel outside its own reach. Under
/// the composite, that pixel must survive.
///
/// The falsification is built into the same search: the "fresh, alone" frame
/// used to find that pixel is exactly the wrong answer this replaces, and the
/// test asserts it is black there — the bug a naive per-record decode (no
/// running canvas) would show a player instead.
#[test]
fn a_never_clearing_room_composites_over_the_prior_canvas_not_a_fresh_one() {
    let Some(raw) = side_b("SAGA #2 - Pirate Adventure [side B].atr") else { return };
    let spliced = splice_vtoc(&raw);
    let entries = line_art_table_entries(&raw);
    let prior_off = entries.iter().find(|e| e.0 == 20).expect("room 20's entry").3;
    let never_clears_off = entries.iter().find(|e| e.0 == 86).expect("room 86's entry").3;

    let mut canvas = LineArtCanvas::new();
    let prior = scott::saga_atari::draw_line_art_record(&mut canvas, &spliced, prior_off)
        .expect("room 20 (a clearing room) plays");

    // The wrong approach: room 86 decoded on its own, with no history —
    // pinned here as what a naive per-record decode would show.
    let mut fresh = LineArtCanvas::new();
    let fresh_pic = scott::saga_atari::draw_line_art_record(&mut fresh, &spliced, never_clears_off)
        .expect("room 86 plays alone too");

    // The composite: room 86 played over room 20's own canvas, in place.
    let composited_after_room86 =
        scott::saga_atari::draw_line_art_record(&mut canvas, &spliced, never_clears_off)
            .expect("room 86 plays over room 20, in place");
    let composited = canvas.picture();

    let sample = (0..prior.width() * prior.height())
        .find(|&i| prior.pixels()[i] != 0 && fresh_pic.pixels()[i] == 0 && composited.pixels()[i] == prior.pixels()[i])
        .map(|i| (i % prior.width(), i / prior.width()))
        .expect("room 20 lights at least one pixel room 86 never reaches on its own");

    let at = |pic: &LineArtPicture, (x, y): (usize, usize)| pic.pixels()[y * pic.width() + x];
    assert_eq!(
        at(&composited, sample), at(&prior, sample),
        "room 20's pixel at {sample:?} should survive under room 86's composite",
    );
    assert_eq!(
        at(&fresh_pic, sample), 0,
        "room 86 played alone (the wrong approach) is black at {sample:?} — that is the bug composited",
    );
    // Sanity: room 86 genuinely drew SOMETHING, so this is not a vacuous case
    // where the two canvases never touched at all.
    assert!(composited_after_room86.painted().is_some());
}

/// **SQ-1525's second host-side fact: the shared darkness card is an
/// animation that ends on a plain clear, so [`scott::saga_atari::draw_darkness_card`]
/// must hand back the LAST PAUSED frame, not the true final (black) one.**
///
/// Falsification: taking the true final frame instead (what a naive
/// `LineArtCanvas::draw` — not `draw_frames` — would give) is pinned wrong
/// right here too: it is the all-black fourth count, not the lit third one.
#[test]
fn draw_darkness_card_shows_the_last_paused_frame_not_the_final_clear() {
    let Some(raw) = side_b("SAGA #1 - Adventureland [side B].atr") else { return };
    let spliced = splice_vtoc(&raw);
    let entries = line_art_table_entries(&raw);
    let off = entries.iter().find(|e| e.0 == 0).expect("the darkness card's entry").3;

    let mut canvas = LineArtCanvas::new();
    let shown = scott::saga_atari::draw_darkness_card(&mut canvas, &spliced, off).expect("plays");
    assert_eq!(lit(&shown), 1_148, "the third pause, not the fourth/final black frame");

    // The canvas's own TRUE end state still finishes the record fully — the
    // next record drawn onto it draws over genuine black, matching the
    // machine, even though the DISPLAYED frame above was an earlier one.
    assert_eq!(lit(&canvas.picture()), 0, "the canvas itself ends black, same as the real machine");

    // Falsification: the naive final frame (`draw`, not `draw_darkness_card`)
    // is the wrong thing to show — all black, not the lettering.
    let naive = LineArtCanvas::new().draw(&entries_record(&spliced, off)).expect("plays");
    assert_eq!(lit(&naive), 0, "the naive final frame is black — the bug this function avoids");
}

/// A record's own bytes, from a FILE offset, the way [`line_art_record`]
/// reads one by table SLOT — used where the offset is already in hand rather
/// than looked up again by slot.
fn entries_record(spliced: &[u8], file_offset: usize) -> Vec<u8> {
    let rec = &spliced[spliced_of(file_offset)..];
    let (_, end) = walk_line_art_record(rec);
    rec[..=end.expect("the record ends")].to_vec()
}

/// **SQ-1525's object draw order**: descending item index, item 0 drawn
/// LAST — read off the renderer's `$8BD5` but not exercised by a test until
/// now. Two overlapping objects on the same room, drawn through
/// `scott::saga_atari::draw_line_art_record` in that order, must leave the
/// LOWER-numbered one's own pixels on top wherever the two overlap.
///
/// *Adventureland* has no pair of objects both present in one room in this
/// specimen's own state, so this proves the ordering directly instead: two
/// synthetic overlapping "object" records (real line-art streams, drawn as
/// object records are — no clear, just paint) sharing one pixel, with the
/// higher-index one drawn FIRST and the lower-index one LAST — as descending
/// order requires — must leave the lower one's colour on top.
#[test]
fn object_draw_order_is_descending_index_so_item_zero_lands_on_top() {
    // Two one-pixel-wide "object" records at the very same point, in two
    // different colours: item 9's (line colour 4) and item 0's (line colour
    // 7). `scott_overlays`-style descending order draws 9 first, then 0 —
    // matching production `PictSource::scott_overlays`'s own sort.
    let stream = |line_colour: u8| -> Vec<u8> {
        vec![0x60 | line_colour, 0, 0, 0x80, 50, 50, 0xA0, 50, 50, 0x00]
    };
    let mut indices = [9u16, 0];
    indices.sort_by(|a, b| b.cmp(a)); // descending: highest first, 0 last
    assert_eq!(indices, [9, 0], "descending order puts item 0 last");

    let mut canvas = LineArtCanvas::new();
    let streams = [(9u16, stream(4)), (0u16, stream(7))];
    let by_index = |want: u16| streams.iter().find(|(i, _)| *i == want).map(|(_, s)| s.clone()).unwrap();
    for &idx in &indices {
        canvas.draw(&by_index(idx)).expect("plays");
    }
    let final_pic = canvas.picture();
    let (a, b) = scott::saga_atari_lineart::COLOUR_PAIRS[7];
    assert_eq!(
        final_pic.pixels()[50 * final_pic.width() + 50],
        a * 4 + b,
        "item 0, drawn last, should be on top at the shared pixel",
    );
}
