//! Macintosh `FONT` / `NFNT` bitmap fonts, as the v6 releases carry them (SQ-0911).
//!
//! Every Macintosh v6 title — *Arthur*, *Journey*, *Shogun*, *Zork Zero* — keeps two
//! of these in its resource fork, and *Beyond Zork* keeps one. MEASURED off
//! `/MAC/ARTHUR FOLDER/ARTHUR`:
//!
//! | resource | cell | advance | what it is |
//! |---|---|---|---|
//! | `FONT` 524 | 7×15 | 6, 7 | the body TYPEFACE — this is the one drawn with |
//! | `FONT` 1033 | 7×12 | 6 only | the **font-3 graphics set**, not a typeface |
//!
//! A `FONT` id encodes family × 128 + point size, so 524 is family 4 at 12pt and
//! 1033 is family 8 at 9pt; id ≡ 0 (mod 128) is the family-name record and carries
//! no bitmap, which is why [`parse`] refuses a zero-length resource rather than
//! treating it as a broken font.
//!
//! **`FONT` 1033 is a graphics set and [`from_fork`] deliberately passes it over**
//! (SQ-1017). `mac/xzip.lst`'s `ZFont` maps `ZALT` to `TextFont (8)`, which is
//! family 8, which is this resource — and dumping its bitmaps confirms it from the
//! other side: code 54 is a solid block, 40 a single column, 38 a bar on one row,
//! 65 a lower-left quadrant. Those are Version 6 font-3 shapes and no letterform is
//! among them. An earlier revision of this header called it a font "worth drawing
//! with" that "drops into the cell model cleanly", which was wrong twice: it is not
//! text, and it does not fit — advance 6 against `colWidth` 7, height 12 against
//! `lineHeight` 15, so it tiles into no Macintosh grid we can observe.
//!
//! What no evidence yet shows is any story USING it. Journey prints font-3 rules
//! and is the obvious place to look; on `machine-screenshots/mac-journey.png` its
//! menu strip runs on a 15-row line and its vertical rules are UNBROKEN over 76
//! pixels, where a 12-tall glyph would gap 3px per line. So those rules do not come
//! from this face at the text line height. `mac-arthur.png` shows no font 3 at all —
//! that frame's border and knotwork are artwork and its status bar is inverse body
//! text. SQ-1017 is deferred on exactly this: we know what the resource is and not
//! what it was for.
//!
//! The Amiga's fonts are the other half of the corpus and split the same way, which
//! is worth stating because the two machines are NOT one argument: *Arthur*'s
//! `char.data` is a proportional 10×10 TYPEFACE (SQ-0916 has a rendered comparison
//! showing it reads worse centred in lanthorn's fixed cell than the public-domain
//! face it would replace), while *Journey*'s `Char.data` is an 8×8 font-3 SET —
//! exactly [`crate::bitmap_font`]-sized, so unlike this one it could blit 1:1.
//!
//! # Format
//!
//! A 26-byte header, then the **strike**: one wide bitmap holding every glyph side
//! by side, `rowWords` words per row, `fRectHeight` rows. Behind it a location table
//! of `lastChar - firstChar + 3` words — glyph *n* occupies strike columns
//! `loc[n]..loc[n+1]`, so a glyph's width is the difference and a zero difference
//! means "not in this font". Behind that the offset/width table, a byte pair per
//! character, `0xFFFF` for a character the font does not define. The offset byte is
//! the left side bearing (added to `kernMax`), and [`parse`] applies it, so a glyph's
//! rows are already positioned inside its advance rather than flush left.
//!
//! The last two entries are the "missing character" glyph and the terminator, which
//! is why the tables carry two more entries than the font has characters.

use crate::bitmap_font::{BitmapFont, Glyph};

/// Header size, and so the offset of the strike.
const HEADER: usize = 26;

/// Parse a `FONT` or `NFNT` resource, or `None` when the bytes are not one.
///
/// The glyph rows come back MSB-leftmost and at most 8 columns wide. A font whose
/// glyphs are wider is refused rather than truncated; every one measured here is 7.
pub fn parse(raw: &[u8]) -> Option<BitmapFont> {
    let be16 = |o: usize| -> Option<u16> { Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?])) };
    let first = usize::from(be16(2)?);
    let last = usize::from(be16(4)?);
    let rect_w = be16(12)?;
    let rect_h = usize::from(be16(14)?);
    let kern_max = i16::from_be_bytes([*raw.get(8)?, *raw.get(9)?]);
    let ow_loc = usize::from(be16(16)?);
    let ascent = be16(18)?;
    let row_words = usize::from(be16(24)?);

    // Bounds that only a decoding error, or a family-name record with no bitmap,
    // could fail. `first` past `last` and a zero-height cell are the giveaways.
    if last < first || last > 255 || rect_h == 0 || rect_h > 32 || row_words == 0 || rect_w == 0 || rect_w > 8 {
        return None;
    }
    let strike = row_words.checked_mul(2)?.checked_mul(rect_h)?;
    if HEADER.checked_add(strike)? > raw.len() {
        return None;
    }
    // `owTLoc` counts WORDS from its own field, which sits at offset 16.
    let ow_table = 16usize.checked_add(ow_loc.checked_mul(2)?)?;
    let n = last.checked_sub(first)?.checked_add(3)?; // +2 sentinels, +1 inclusive
    let loc_table = ow_table.checked_sub(n.checked_mul(2)?)?;

    let mut glyphs = Vec::with_capacity(n);
    for i in 0..n - 1 {
        let (lo, hi) = (be16(loc_table + i * 2)?, be16(loc_table + i * 2 + 2)?);
        let img_w = usize::from(hi.checked_sub(lo)?);
        // The advance is the offset/width table's width byte; `0xFF` marks a
        // character the font does not define, which draws and advances as nothing.
        let obyte = *raw.get(ow_table + i * 2)?;
        let wbyte = *raw.get(ow_table + i * 2 + 1)?;
        let advance = if wbyte == 0xFF { 0 } else { wbyte };
        // The LEFT SIDE BEARING, which is `kernMax + owOffset` and is what places a
        // narrow glyph inside its advance. Dropping it flushes every glyph left and
        // makes evenly-advanced text look raggedly letter-spaced — that mistake is
        // what made the first SQ-0916 preview of this font read far worse than it is.
        let bearing = if obyte == 0xFF {
            0
        } else {
            usize::try_from(i32::from(kern_max) + i32::from(obyte)).unwrap_or(0)
        };
        if img_w.checked_add(bearing)? > 8 {
            return None;
        }
        let mut rows = Vec::with_capacity(rect_h);
        for y in 0..rect_h {
            let base = HEADER + y * row_words * 2;
            let mut bits = 0u8;
            for x in 0..img_w {
                let bit = usize::from(lo) + x;
                if *raw.get(base + bit / 8)? & (0x80 >> (bit % 8)) != 0 {
                    bits |= 0x80 >> (x + bearing);
                }
            }
            rows.push(bits);
        }
        glyphs.push(Glyph { width: advance, rows });
    }
    // The final two entries are the missing-character glyph and the terminator;
    // neither stands for a code, so neither belongs in the table.
    glyphs.truncate(last - first + 1);
    Some(BitmapFont {
        width: u8::try_from(rect_w).ok()?,
        height: u8::try_from(rect_h).ok()?,
        baseline: u8::try_from(ascent).unwrap_or(0),
        // A `FONT` resource has no `tf_BoldSmear` equivalent — the Macintosh's own
        // bold is synthesised, so there is no stored width to widen by (SQ-1009).
        bold_smear: 0,
        proportional: BitmapFont::measure_proportional(&glyphs),
        lo: u8::try_from(first).ok()?,
        glyphs,
    })
}
/// The body face off a mounted Macintosh volume (SQ-1011).
///
/// Mirrors [`crate::amiga_font::from_volume`]: hand it the volume, get the
/// typeface the release shipped. An Infocom Macintosh release is **all resource
/// fork** — the application's data fork is zero bytes — so the font is reachable
/// only through [`crate::hfs::Hfs::read_resource`], which is why this exists
/// rather than callers walking files themselves.
///
/// Searches the `APPL` entries, since that is where Infocom put `FONT` 524; a
/// volume with no application, no fork, or no bitmap `FONT` answers `None`.
///
/// **This asks "what face is on this disk", which on a compilation is the wrong
/// question** — prefer [`from_volume_beside`], which asks for one story's own.
/// It used to take the FIRST `APPL` and stop, and on the Masterpieces CD that is
/// `A MIND FOREVER VOYAGING`, a Version 4 title shipping no `FONT` at all: every
/// graphical game on the platter resolved to no face and drew its 7x15 cell with
/// the 8-wide fallback (SQ-1018). Scanning all of them at least answers with a
/// face that is on the disc, but it is still not necessarily *this game's*.
///
/// # What it is for
///
/// The Macintosh's Version 6 cell is 7x15 (`mac/xzip.lst`'s `colWidth := 7;
/// lineHeight := 15`), and this face is drawn for exactly that — so it blits 1:1
/// into that cell where a face drawn for an 8-pixel advance has nowhere to keep
/// its inter-character gap. Note the METRIC and the FACE are separate facts: the
/// interpreter declared 7 while painting proportional Geneva, and this resource
/// is the fixed-pitch face it shipped.
pub fn from_volume(hfs: &crate::hfs::Hfs) -> Option<BitmapFont> {
    forks_in(hfs, |_| true).filter_map(|rf| from_fork(&rf)).max_by_key(|f| f.height)
}

/// The face shipped **beside** the story at `path` — same folder — or `None`
/// when that story's own application carries none (SQ-1018).
///
/// [`crate::hfs::Hfs::pictures_beside`]'s rule, applied to the typeface, and for
/// the identical reason: a volume that keeps its games in folders can answer for
/// one game, and a compilation is exactly where "on this disk" and "beside this
/// story" stop being the same question. SQ-0876 fixed the artwork half after
/// every graphical game on the Masterpieces CD was handed Zork Zero's plates;
/// the font half had the same shape and went unnoticed because its failure is
/// silent — a missing face falls back to a legible-but-wrong one rather than
/// drawing the wrong game's pictures.
///
/// **A name this volume does not hold falls through to [`from_volume`]**, which
/// is what keeps a single-game floppy working: there the story and the
/// application both sit at the root, so "beside" and "on this disk" already
/// describe the same file set, and nothing about that case moves.
///
/// Unlike `pictures_beside`, a folder with no font does NOT stop the search.
/// Artwork is the game's own or it has none; a typeface is the *machine's*, and
/// every Macintosh v6 release ships the identical `FONT` 524 — so falling
/// through to the disc's other applications yields the same 2906-byte resource
/// rather than another game's plates. The pairing is still tried first, because
/// it is the honest question and it is what makes the answer right when the
/// releases ever do differ.
pub fn from_volume_beside(hfs: &crate::hfs::Hfs, path: &str) -> Option<BitmapFont> {
    let Some(story) = hfs.files().iter().find(|e| e.path().eq_ignore_ascii_case(path)) else {
        return from_volume(hfs);
    };
    let dirs = story.dirs.clone();
    forks_in(hfs, move |e| e.dirs == dirs)
        .filter_map(|rf| from_fork(&rf))
        .max_by_key(|f| f.height)
        .or_else(|| from_volume(hfs))
}

/// Every face in one fork, with the resource id it is stored under.
///
/// [`from_fork`] answers with the ONE face worth drawing body text in; this
/// keeps them all, and their ids, because an id is what distinguishes them to a
/// reader: it encodes family × 128 + point size, so `524` is family 4 at 12pt
/// and `1033` is family 8 at 9pt — which `mac/xzip.lst`'s `ZFont` selects as
/// `ZALT`. A listing that collapsed them would hide that a release ships two.
pub fn faces_in_fork(fork: &crate::resource_fork::ResourceFork) -> Vec<(i16, BitmapFont)> {
    fork.of_type(b"FONT")
        .iter()
        .chain(fork.of_type(b"NFNT"))
        .filter_map(|r| parse(&r.data).map(|f| (r.id, f)))
        .collect()
}

/// Every face the application beside the story at `path` carries, with ids.
///
/// [`from_volume_beside`]'s pairing, reporting rather than choosing — for a
/// surface that shows a person what a medium holds. Same fallback, so the two
/// cannot disagree about which application they are reading.
pub fn faces_beside(hfs: &crate::hfs::Hfs, path: &str) -> Vec<(i16, BitmapFont)> {
    let all = |h: &crate::hfs::Hfs| -> Vec<(i16, BitmapFont)> {
        forks_in(h, |_| true).flat_map(|rf| faces_in_fork(&rf)).collect()
    };
    let Some(story) = hfs.files().iter().find(|e| e.path().eq_ignore_ascii_case(path)) else {
        return all(hfs);
    };
    let dirs = story.dirs.clone();
    let beside: Vec<_> =
        forks_in(hfs, move |e| e.dirs == dirs).flat_map(|rf| faces_in_fork(&rf)).collect();
    if beside.is_empty() { all(hfs) } else { beside }
}

/// Every resource fork reachable through an `APPL` this volume holds that `pick`
/// accepts, in catalog order.
fn forks_in<'a>(
    hfs: &'a crate::hfs::Hfs,
    pick: impl Fn(&crate::hfs::HfsEntry) -> bool + 'a,
) -> impl Iterator<Item = crate::resource_fork::ResourceFork> + 'a {
    hfs.files()
        .iter()
        .filter(move |e| e.file_type == *b"APPL" && pick(e))
        .filter_map(|e| hfs.read_resource(e))
        .filter_map(|fork| crate::resource_fork::ResourceFork::parse(&fork))
}


/// The best font in a resource fork, or `None` when it holds none.
///
/// "Best" is the tallest cell, because that is the one designed for reading: the two
/// a v6 release carries are a 7×15 and a 7×12, and the taller is the body face.
/// `NFNT` is checked as well as `FONT` — they are the same payload under two type
/// codes, and a release could use either.
pub fn from_fork(fork: &crate::resource_fork::ResourceFork) -> Option<BitmapFont> {
    fork.of_type(b"FONT")
        .iter()
        .chain(fork.of_type(b"NFNT"))
        .filter_map(|r| parse(&r.data))
        .max_by_key(|f| f.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hfs::Hfs;
    use crate::resource_fork::tests::{FORK_LEN, VOLUME, fork_bytes};

    /// Every glyph row this module expects, written out by hand.
    ///
    /// The fixture's art is five columns wide and its left side bearing is 1
    /// (`kernMax` -1 plus an offset byte of 2), so column *x* of the image
    /// lands in bit `0x80 >> (x + 1)` — the five-bit pattern shifted left by
    /// two. `.###.` is `0b01110 << 2` = `0x38`, `#...#` is `0b10001 << 2` =
    /// `0x44`, `#####` is `0x7C`, `####.` is `0x78`, `#....` is `0x40`.
    ///
    /// **A reader that ignored `kernMax` would produce every one of these
    /// shifted one column right**, which is why the fixture uses a non-zero
    /// one: the whole table is the bearing's falsification test.
    mod art {
        //   .....  x3
        //   .###.  #...#  #...#  #...#  #####  #...#  #...#  #...#  #...#
        //   .....  x3
        pub const A: [u8; 15] =
            [0, 0, 0, 0x38, 0x44, 0x44, 0x44, 0x7C, 0x44, 0x44, 0x44, 0x44, 0, 0, 0];
        //   ####.  #...#  #...#  #...#  ####.  #...#  #...#  #...#  ####.
        pub const B: [u8; 15] =
            [0, 0, 0, 0x78, 0x44, 0x44, 0x44, 0x78, 0x44, 0x44, 0x44, 0x78, 0, 0, 0];
        //   .###.  #...#  #....  #....  #....  #....  #....  #...#  .###.
        pub const C: [u8; 15] =
            [0, 0, 0, 0x38, 0x44, 0x40, 0x40, 0x40, 0x40, 0x40, 0x44, 0x38, 0, 0, 0];
        /// `D` is empty by design — zero image width, advance 3.
        pub const D: [u8; 15] = [0; 15];

        //   .###.  #...#  #...#  #...#  #...#  #...#  #...#  .###.
        pub const ZERO: [u8; 12] = [0, 0, 0x38, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x38, 0, 0];
        //   ..#..  .##..  ..#..  ..#..  ..#..  ..#..  ..#..  .###.
        pub const ONE: [u8; 12] = [0, 0, 0x10, 0x30, 0x10, 0x10, 0x10, 0x10, 0x10, 0x38, 0, 0];
    }

    fn volume() -> Hfs {
        Hfs::mount(VOLUME.to_vec()).expect("the synthetic volume mounts")
    }

    /// The whole chain SQ-1015 was raised to cover, in one case, on a fixture
    /// that is present on CI.
    ///
    /// `Hfs::mount` → the `APPL` entry → `read_resource` off a file whose DATA
    /// fork is zero bytes → `ResourceFork::parse` → `mac_font::parse`. Every
    /// step is asserted, so a break anywhere in it names itself.
    #[test]
    fn reads_a_font_off_a_synthetic_macintosh_volume() {
        let hfs = volume();
        assert_eq!(hfs.volume_name(), "Lanthorn Test");
        assert_eq!(hfs.files().len(), 2, "a story and an application, both in a folder");

        let app = hfs
            .files()
            .iter()
            .find(|e| e.file_type == *b"APPL")
            .expect("the application is in the catalog");
        assert_eq!(app.path(), "Test Folder/TestApp");
        assert_eq!(app.size, 0, "an Infocom Macintosh application has NO data fork");
        assert_eq!(hfs.read(app).as_deref(), Some(&[][..]), "and reads as an empty file");
        assert_eq!(app.resource_size, FORK_LEN, "everything is in the resource fork");

        let raw = hfs.read_resource(app).expect("the resource fork is reachable");
        assert_eq!(raw, fork_bytes(), "and is exactly the bytes at allocation block 2");

        let fork = crate::resource_fork::ResourceFork::parse(&raw).expect("a resource fork");
        let font = fork.get(b"FONT", 524).expect("FONT 524");
        let face = parse(&font.data).expect("FONT 524 parses");
        assert_eq!((face.width, face.height, face.baseline), (7, 15, 12));
    }

    /// A file with no resource fork answers `None` rather than an empty one —
    /// the distinction `read_resource` exists to make.
    #[test]
    fn a_file_with_no_resource_fork_says_so() {
        let hfs = volume();
        let story = hfs
            .files()
            .iter()
            .find(|e| e.file_type == *b"INdf")
            .expect("the story is in the catalog");
        assert_eq!(story.resource_size, 0);
        assert_eq!(hfs.read_resource(story), None);
        assert_eq!(hfs.read(story).map(|b| b.len()), Some(512), "its data fork is there");
    }

    /// Every field and every glyph of `FONT` 524, against values written out
    /// from the format description rather than read back off this parser.
    #[test]
    fn parses_the_font_resource_glyph_for_glyph() {
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        let face = parse(&fork.get(b"FONT", 524).expect("FONT 524").data).expect("parses");

        assert_eq!(face.width, 7, "fRectWidth");
        assert_eq!(face.height, 15, "fRectHeight");
        assert_eq!(face.baseline, 12, "ascent");
        assert_eq!(face.lo, 0x41, "firstChar");
        assert_eq!(
            face.glyphs.len(),
            4,
            "firstChar..=lastChar — the missing-character glyph and the terminator are NOT codes"
        );

        for (code, rows) in [(b'A', art::A), (b'B', art::B), (b'C', art::C), (b'D', art::D)] {
            let g = face.glyph(code).unwrap_or_else(|| panic!("{} is in the font", code as char));
            assert_eq!(g.rows.as_slice(), &rows[..], "the bitmap for {}", code as char);
        }
        assert_eq!(face.glyph(b'A').map(|g| g.width), Some(7));
        assert_eq!(face.glyph(b'D').map(|g| g.width), Some(3), "a narrow blank, like a space");
        assert!(face.glyph(b'E').is_none(), "past lastChar");
        assert!(face.glyph(b'@').is_none(), "before firstChar");
    }

    /// A blank glyph with an odd advance must not make a fixed-pitch font read
    /// as proportional — the fixture's `D` is exactly that, and a reader that
    /// counted it would answer `true` here.
    #[test]
    fn a_blank_glyph_does_not_make_a_fixed_font_proportional() {
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        let face = parse(&fork.get(b"FONT", 524).expect("FONT 524").data).expect("parses");
        assert!(face.glyph(b'D').is_some_and(|g| g.rows.iter().all(|&r| r == 0)));
        assert_ne!(face.glyph(b'D').map(|g| g.width), face.glyph(b'A').map(|g| g.width));
        assert!(!face.proportional, "the three glyphs that are DRAWN all advance 7");
    }

    /// The second face, which is shorter — so `from_fork`'s "tallest wins"
    /// rule has something to choose between, on a volume rather than in the
    /// abstract.
    #[test]
    fn parses_the_second_face_too() {
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        let face = parse(&fork.get(b"FONT", 1033).expect("FONT 1033").data).expect("parses");
        assert_eq!((face.width, face.height, face.baseline, face.lo), (7, 12, 9, 0x30));
        assert_eq!(face.glyphs.len(), 2);
        assert_eq!(face.glyph(b'0').map(|g| g.rows.as_slice()), Some(&art::ZERO[..]));
        assert_eq!(face.glyph(b'1').map(|g| g.rows.as_slice()), Some(&art::ONE[..]));
        assert_eq!(face.glyph(b'0').map(|g| g.width), Some(6));
        assert!(!face.proportional);
    }

    #[test]
    fn from_fork_and_from_volume_pick_the_taller_face() {
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        assert_eq!(from_fork(&fork).map(|f| f.height), Some(15), "15 over 12");
        assert_eq!(from_volume(&volume()).map(|f| f.height), Some(15));
    }

    #[test]
    fn faces_in_fork_reports_both_with_their_ids() {
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        let faces = faces_in_fork(&fork);
        assert_eq!(
            faces.iter().map(|(id, f)| (*id, f.height)).collect::<Vec<_>>(),
            vec![(524, 15), (1033, 12)],
            "in map order, and a listing must not collapse the two"
        );
    }

    /// The application and the story share a folder, so "beside this story" and
    /// "on this disk" reach the same face here — and a name the volume does not
    /// hold falls through rather than answering nothing.
    #[test]
    fn from_volume_beside_pairs_the_application_with_the_story() {
        let hfs = volume();
        assert_eq!(from_volume_beside(&hfs, "Test Folder/Story.data").map(|f| f.height), Some(15));
        assert_eq!(from_volume_beside(&hfs, "TEST FOLDER/STORY.DATA").map(|f| f.height), Some(15));
        assert_eq!(from_volume_beside(&hfs, "Nowhere/Absent.data").map(|f| f.height), Some(15));
        assert_eq!(
            faces_beside(&hfs, "Test Folder/Story.data")
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            vec![524, 1033]
        );
    }

    /// A `FONT` id that is a multiple of 128 is the family-NAME record and
    /// carries no bitmap, so `parse` must refuse it rather than invent a font.
    #[test]
    fn refuses_resources_that_are_not_fonts() {
        assert_eq!(parse(&[]), None, "a family-name record is zero-length");
        assert_eq!(parse(&[0u8; 26]), None, "an all-zero header has no cell");
        let fork = crate::resource_fork::ResourceFork::parse(fork_bytes()).expect("a fork");
        let good = &fork.get(b"FONT", 524).expect("FONT 524").data;
        // The offset/width table starts at 98 — `owTLoc` 41 words past offset
        // 16 — so cutting there leaves the header and the strike intact and
        // takes away the table the glyph widths come from.
        assert_eq!(parse(&good[..98]), None, "the offset/width table is cut off");
        assert_eq!(parse(&good[..40]), None, "the strike is cut off");
        // lastChar before firstChar is the other giveaway of a mis-decode.
        let mut swapped = good.clone();
        swapped[2..4].copy_from_slice(&0x0050u16.to_be_bytes());
        assert_eq!(parse(&swapped), None);
    }
}
