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

use scott::saga_atari::{
    decode_line_art_opening, decode_record, scan_picture_side, splice_vtoc, AtariRecord, SIDE_LEN,
};
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

/// SQ-1497: the picture that opens every **line-art** side, at
/// `LINE_ART_OFFSET` — confirming the premise the quest states (§8.4's item
/// 26 grammar reproduces here, byte for byte) rather than re-deriving it.
///
/// **The same picture, byte for byte, on all four titles.** Painted box
/// (54,50)-(140,96), 725 green pixels (`PALETTE`'s index 2) over the white
/// ground and nothing else — so this is not any one title's own art (four
/// different games cannot share one room's picture), which is exactly why
/// `crate::saga_atari`'s module docs stop here rather than claiming it answers
/// for a room or for the boot title card.
#[test]
fn the_line_art_opening_is_the_same_small_picture_on_every_title() {
    for file in LINE_ART_TITLES {
        let Some(raw) = side_b(file) else { continue };
        let pic = decode_line_art_opening(&raw).unwrap_or_else(|e| panic!("{file}: {e:?}"));
        // Family D's own canvas (280x192), not family C's (280x160) the rest
        // of this file's `CANVAS_WIDTH`/`CANVAS_HEIGHT` imports name.
        assert_eq!(
            (pic.width(), pic.height()),
            (scott::apple_pictures::CANVAS_WIDTH, scott::apple_pictures::CANVAS_HEIGHT),
        );
        let painted = pic.painted().unwrap_or_else(|| panic!("{file}: the opening picture paints nothing"));
        assert_eq!(
            (painted.left(), painted.top(), painted.right(), painted.bottom()),
            (54, 50, 140, 96),
            "{file}: the opening picture's own bounding box",
        );
        let mut counts = [0usize; 6];
        for &v in pic.pixels() {
            counts[usize::from(v)] += 1;
        }
        assert_eq!(counts, [0, 0, 725, 0, 0, 53_035], "{file}: green pixels over the white ground");
    }
}

/// Offset precision matters: one byte off `LINE_ART_OFFSET` and the decode
/// gives a DIFFERENT picture, not a refusal — this format has no header to
/// bounds-check against, so a wrong offset decodes silently rather than
/// erroring, and this is the falsification that says the exact offset is
/// load-bearing rather than approximately right.
#[test]
fn one_byte_off_line_art_offset_decodes_a_different_picture() {
    let file = "SAGA #1 - Adventureland [side B].atr";
    let Some(raw) = side_b(file) else { return };
    let at_offset = decode_line_art_opening(&raw).expect("decodes at LINE_ART_OFFSET");
    // Shift the WHOLE buffer back by one byte, so `decode_line_art_opening`'s
    // own `LINE_ART_OFFSET` addition lands one byte later in the real data
    // than it should — the same effect a wrong constant would have.
    let shifted = decode_line_art_opening(&raw[1..]).expect("still decodes, just wrong");
    let non_white = |pic: &scott::apple_pictures::HiResPicture| {
        pic.pixels().iter().filter(|&&v| v != 5).count()
    };
    assert_eq!(non_white(&at_offset), 725, "the real picture, pinned above");
    assert_ne!(non_white(&shifted), non_white(&at_offset), "one byte off reads different bytes as tokens");
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
