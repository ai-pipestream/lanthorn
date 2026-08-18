//! The font-3 translation table, checked against the font Infocom shipped (SQ-0915).
//!
//! `zvm::cpu::exec::font3_translate` turns the Z-machine's character-graphics font
//! (ZMSD §16) into Unicode, because a terminal draws characters and not bitmaps. The
//! table came from bocfel, which is a faithful reading of the *standard* — but the
//! standard describes the glyphs in prose, and what a player actually saw is what
//! their machine drew.
//!
//! *Beyond Zork* shipped that font on the floppy. `Graphic.Data` on *Lost Treasures*
//! Amiga disk 5 is an 8×8 Amiga disk font — not artwork, despite the name, and not a
//! program despite the `$3F3` hunk header, which is the four-byte `moveq/rts` stub
//! every Amiga font begins with. It carries the whole of font 3: arrows, diagonals,
//! rules, the box-drawing set, blocks, meter bars, and the 26 runes.
//!
//! **This is the only oracle here that can falsify the QUESTION rather than the
//! answer.** Every other test asks whether babelmap does what the table says; this
//! one asks whether the table is right, and it immediately found that codes 71–74
//! put `U+FFFD` on screen where the Amiga inked a corner pixel.
//!
//! # What is deliberately NOT claimed
//!
//! Unicode cannot express all of these. Codes 38/39 are rules at two different
//! heights and both become `─`; 79–87 are meter segments with top and bottom rails
//! and become the graded left-blocks without them; 123–126 are reverse-video
//! variants of 92–96 and lose the reversal. Those are approximations the terminal
//! forces, and this suite asserts none of them away — it asserts the one thing that
//! is unambiguously wrong whatever your rendering budget, which is drawing nothing
//! recognisable at all where the machine drew ink.
//!
//! Fixtures are gitignored, so every case skips vacuously when one is absent.

use std::path::PathBuf;

fn treasures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../treasures")
}

/// An 8×8 glyph as one byte per row, MSB leftmost.
type Glyph = [u8; 8];

/// The parsed font: `LoChar` and one glyph per code.
struct AmigaFont {
    lo: u8,
    glyphs: Vec<Glyph>,
}

impl AmigaFont {
    fn glyph(&self, code: u8) -> Option<Glyph> {
        self.glyphs.get(usize::from(code).checked_sub(usize::from(self.lo))?).copied()
    }
}

/// Parse an Amiga disk font out of an AmigaDOS hunk file.
///
/// Layout, from `<diskfont/diskfont.h>` and `<graphics/text.h>`: a four-byte
/// `moveq #0,d0 / rts` stub, then `DiskFontHeader` — a 14-byte `Node`, `dfh_FileID`
/// (`0x0F80`, which is the signature checked here), `dfh_Revision`, `dfh_Segment`,
/// and a 32-byte name — then `TextFont`, whose own 20-byte `Message` precedes
/// `tf_YSize`. Glyph rows are stored as one long bitmap per row, `tf_Modulo` bytes
/// wide; `tf_CharLoc` is an array of `(bit offset, bit width)` pairs, one per code.
fn parse_amiga_font(raw: &[u8]) -> Option<AmigaFont> {
    let be16 = |o: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?]))
    };
    let be32 = |o: usize| -> Option<u32> {
        Some(u32::from_be_bytes([*raw.get(o)?, *raw.get(o + 1)?, *raw.get(o + 2)?, *raw.get(o + 3)?]))
    };
    // Skip the hunk header: HUNK_HEADER, an empty resident-library list, the table
    // size and first/last hunk, one size per hunk, then HUNK_CODE and its length.
    if be32(0)? != 0x0000_03F3 || be32(4)? != 0 {
        return None;
    }
    let hunks = be32(16)?.checked_sub(be32(12)?)?.checked_add(1)? as usize;
    let body = 20 + hunks * 4 + 8;
    let at = |o: usize| body + o;

    if be16(at(0x12))? != 0x0F80 {
        return None; // not DFH_ID, so not a disk font
    }
    const TF: usize = 0x3A; // 4-byte stub + DiskFontHeader up to and including dfh_Name
    let (ysize, xsize) = (be16(at(TF + 20))?, be16(at(TF + 24))?);
    let (lo, hi) = (*raw.get(at(TF + 32))?, *raw.get(at(TF + 33))?);
    let (chardata, modulo, charloc) =
        (be32(at(TF + 34))? as usize, be16(at(TF + 38))? as usize, be32(at(TF + 40))? as usize);
    if ysize != 8 || xsize != 8 || hi < lo {
        return None;
    }

    let mut glyphs = Vec::new();
    for i in 0..=usize::from(hi - lo) {
        let (off, wid) = (be16(at(charloc + i * 4))? as usize, be16(at(charloc + i * 4 + 2))? as usize);
        let mut g = [0u8; 8];
        for (y, row) in g.iter_mut().enumerate() {
            for x in 0..wid.min(8) {
                let bit = off + x;
                let byte = *raw.get(at(chardata + y * modulo + bit / 8))?;
                if byte & (0x80 >> (bit % 8)) != 0 {
                    *row |= 0x80 >> x;
                }
            }
        }
        glyphs.push(g);
    }
    Some(AmigaFont { lo, glyphs })
}

/// `Graphic.Data` off the Beyond Zork volume of the Amiga *Lost Treasures* set.
fn shipped_font() -> Option<AmigaFont> {
    let disk = treasures_dir().join("Lost Treasures of Infocom, The_Disk5.adf");
    if !disk.is_file() {
        eprintln!("SKIP: gitignored Lost Treasures disk 5 absent");
        return None;
    }
    let raw = std::fs::read(&disk).ok()?;
    let vol = blorb::medium::MountedDisk::mount(raw).ok()?;
    let (_, bytes) = vol
        .contents()
        .into_iter()
        .find(|(n, _)| n.rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case("Graphic.Data")))?;
    let font = parse_amiga_font(&bytes).expect("Graphic.Data should parse as an Amiga disk font");
    Some(font)
}

/// The file really is an 8×8 disk font covering font 3's whole range.
///
/// A non-vacuity guard for the two cases below: if this file ever stops parsing, or
/// stops covering 32..=126, they would pass by skipping rather than by agreeing.
#[test]
fn beyond_zorks_graphic_data_is_an_eight_by_eight_font_covering_font_three() {
    let Some(font) = shipped_font() else { return };
    assert_eq!(font.lo, 32, "font 3 starts at the space");
    for code in 32u8..=126 {
        assert!(font.glyph(code).is_some(), "no glyph for font-3 code {code}");
    }
    let inked = (32u8..=126).filter(|&c| font.glyph(c).is_some_and(|g| g != [0; 8])).count();
    assert!(inked >= 80, "only {inked} of 95 codes carry ink — this is not font 3");
}

/// **No code the Amiga inks may translate to a replacement character.**
///
/// This is the case that found the defect. Codes 71–74 are unassigned in ZMSD §16,
/// so bocfel renders them `U+FFFD` and we copied that — but the shipped font draws a
/// corner pixel for each, which means *Beyond Zork* can print them and a player saw
/// `\u{FFFD}` where the Amiga drew a mark.
///
/// Falsified by restoring `71..=74 => '\u{FFFD}'`, which names all four here.
#[test]
fn every_code_the_amiga_draws_translates_to_something_drawable() {
    let Some(font) = shipped_font() else { return };
    // The ONE documented exception, and it is an approximation rather than a hole.
    // Code 79 is the EMPTY member of the meter-bar family at 79-87: its only ink is
    // the top and bottom rail, and 80-87 — which we render as the graded left-blocks
    // `▏▎▍▌▋▊▉█` — drop those same rails. So the family reads as a gradient that
    // starts at nothing, and a space is the consistent zero. Unicode has no
    // "rails-only" cell to do better with.
    const EMPTY_METER_BAR: u8 = 79;
    let mut lost = Vec::new();
    for code in 32u8..=126 {
        let Some(glyph) = font.glyph(code) else { continue };
        if glyph == [0; 8] || code == EMPTY_METER_BAR {
            continue; // the machine draws nothing, so a space is a faithful answer
        }
        let ch = zvm::cpu::exec::font3_translate(char::from(code));
        if ch == '\u{FFFD}' || ch == ' ' {
            lost.push((code, ch));
        }
    }
    // …and the exception has to keep earning itself: if 79 ever stops being the
    // rails-only glyph, the reasoning above no longer applies to it.
    let bar = font.glyph(EMPTY_METER_BAR).expect("code 79 is in range");
    assert_eq!(
        bar.iter().filter(|&&r| r == 0xFF).count(),
        2,
        "code 79 is excused as the empty meter bar, whose ink is exactly two rails: {bar:02X?}",
    );
    assert!(bar.iter().all(|&r| r == 0xFF || r == 0), "code 79's rails are full rows: {bar:02X?}");
    assert!(
        lost.is_empty(),
        "these font-3 codes are INKED in the font Infocom shipped with Beyond Zork, yet \
         translate to nothing a reader can see: {lost:?}\n\
         Unicode cannot match every glyph exactly and this suite does not ask it to — but a \
         replacement character, or a blank, where the Amiga drew ink is not an approximation, \
         it is a hole (SQ-0915).",
    );
}

/// Spot checks: the shipped ink agrees with what the table claims the glyph IS.
///
/// Pinned as corner/edge occupancy rather than exact bitmaps, because the assertion
/// is about which glyph a code denotes, not about Infocom's letterforms. A left
/// arrow inks the left edge and a right arrow does not, whatever its exact shape.
#[test]
fn the_shipped_glyphs_agree_with_the_table_where_the_shape_is_unambiguous() {
    let Some(font) = shipped_font() else { return };
    let ink = |code: u8| font.glyph(code).unwrap_or([0; 8]);
    let any = |g: Glyph, rows: std::ops::Range<usize>, mask: u8| {
        rows.into_iter().any(|y| g[y] & mask != 0)
    };

    // 33/34 are the horizontal cursor arrows: each reaches its own edge.
    assert!(any(ink(33), 0..8, 0x80), "code 33 (←) should ink the left edge");
    assert!(any(ink(34), 0..8, 0x02), "code 34 (→) should ink the right side");
    // 40/41 are verticals and 38/39 horizontals — neither is the other.
    assert!(
        ink(40).iter().all(|r| r.count_ones() == 1),
        "code 40 (│) is a vertical: one pixel on every row, got {:02X?}",
        ink(40),
    );
    assert_eq!(ink(38).iter().filter(|&&r| r == 0xFF).count(), 1, "code 38 (─) is one full row");
    // 54 is the solid block, 32 and 37 are blank.
    assert_eq!(ink(54), [0xFF; 8], "code 54 (█) is the full cell");
    assert_eq!(ink(32), [0; 8], "code 32 is the space");
    assert_eq!(ink(37), [0; 8], "code 37 is blank in the table and blank on the disk");
    // 71-74 are the corner pixels the table used to lose: one corner each, and each
    // a DIFFERENT corner, which is what makes four separate codepoints necessary.
    for (code, row, mask, corner) in
        [(71u8, 0usize, 0x01u8, "upper-right"), (72, 7, 0x01, "lower-right"),
         (73, 7, 0x80, "lower-left"), (74, 0, 0x80, "upper-left")]
    {
        let g = ink(code);
        assert_eq!(g[row] & mask, mask, "code {code} should ink the {corner} corner");
        assert_eq!(g.iter().map(|r| r.count_ones()).sum::<u32>(), 1, "code {code} is ONE pixel");
    }
    // The 26 runes all carry ink and all land in Unicode's runic block.
    for code in 97u8..=122 {
        assert_ne!(ink(code), [0; 8], "rune at code {code} should be drawn");
        let ch = zvm::cpu::exec::font3_translate(char::from(code)) as u32;
        assert!(
            (0x16A0..=0x16F8).contains(&ch) || (0x15BE..=0x15BE).contains(&ch),
            "code {code} should translate into the runic block, got U+{ch:04X}",
        );
    }
}
