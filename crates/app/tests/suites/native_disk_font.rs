//! The bitmap fonts Infocom shipped on the release floppies (SQ-0916).
//!
//! Four Amiga releases carry one, as an AmigaDOS disk font — a file that looks like
//! an executable and is not. `blorb::amiga_font` parses it; these cases pin it
//! against the real floppies, because a font parser that agrees only with its own
//! synthetic fixture agrees with nothing.
//!
//! Fixtures are gitignored, so every case skips vacuously when one is absent.

use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn font_on(disk: &str) -> Option<blorb::amiga_font::AmigaFont> {
    let path = stories_dir().join(disk);
    if !path.is_file() {
        eprintln!("SKIP: gitignored floppy absent: {disk}");
        return None;
    }
    let files: Vec<(String, Vec<u8>)> = app::assets::files(&path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let n = f.name.clone();
            f.into_bytes().map(|b| (n, b))
        })
        .collect();
    let font = blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
    assert!(font.is_some(), "{disk} should carry a font");
    font
}

/// **Arthur ships a proportional 10×10 typeface**, and it is a text font — letters,
/// not the box-drawing set.
///
/// The proportional flag is the load-bearing fact: it means Arthur's Amiga text was
/// never monospaced, so nobody should expect a column-for-column match against an
/// Amiga screenshot, and it is why `Glyph::width` exists separately from the font's
/// nominal width.
#[test]
fn arthur_carries_a_proportional_text_font() {
    let Some(font) = font_on("Arthur - The Quest for Excalibur.adf") else { return };
    assert_eq!((font.width, font.height), (10, 10), "nominal cell");
    assert_eq!(font.baseline, 8);
    assert!(font.proportional, "FPF_PROPORTIONAL is set");
    assert_eq!(font.lo, 32, "starts at the space");
    assert_eq!(font.glyphs.len(), 127 - 32 + 1, "covers 32..=127");

    // A text font, not font 3: 'A' is a letter and the space is blank.
    let a = font.glyph(b'A').expect("'A'");
    assert_eq!(a.rows.len(), 10);
    assert_ne!(a.rows.iter().fold(0, |x, r| x | r), 0, "'A' is drawn");
    assert!(font.glyph(b' ').expect("space").rows.iter().all(|&r| r == 0), "the space is blank");

    // Proportional in fact and not just in flag: the widths really differ.
    let widths: std::collections::BTreeSet<u8> =
        (32u8..=126).filter_map(|c| font.glyph(c)).map(|g| g.width).collect();
    assert!(widths.len() > 3, "a proportional font has several widths, saw {widths:?}");
    assert!(widths.iter().all(|&w| w <= 8), "no glyph is wider than a byte: {widths:?}");
    assert!(font.glyph(b'i').unwrap().width < font.glyph(b'm').unwrap().width, "'i' is narrower than 'm'");

    // Descenders reach the last two rows, which is why an 8-row master would clip.
    for ch in *b"gpqyj" {
        let g = font.glyph(ch).expect("descender");
        assert_ne!(g.rows[9], 0, "{} descends to the last row", char::from(ch));
    }
}

/// **Journey's font is the font-3 set, and it is byte-identical to Beyond Zork's.**
///
/// Same file under two names — `Char.data` on the v6 disk, `Graphic.Data` on the v5
/// one. That is what makes font 3 reachable from the v6 raster path, and it is the
/// reason `bitfont`'s font-3 entries are not dead code (see that module's header).
#[test]
fn journey_carries_the_font_three_set_and_not_a_typeface() {
    let Some(font) = font_on("Journey - The Quest Begins.adf") else { return };
    assert_eq!((font.width, font.height), (8, 8));
    assert!(!font.proportional, "the font-3 set is fixed-pitch");
    assert_eq!(font.lo, 32);
    // Every glyph is the full cell width — that is what fixed-pitch means here.
    assert!(
        (32u8..=126).filter_map(|c| font.glyph(c)).all(|g| g.width == 8),
        "font 3 glyphs are all 8 wide",
    );
    // NOT a typeface: code 65 is a solid block in font 3, not the letter A. Pinned
    // as "the inked rows are all the same run", which is true of a block and false
    // of every letterform — an 'A' has an apex, a crossbar and two legs.
    let a = font.glyph(b'A').expect("code 65");
    let inked: Vec<u8> = a.rows.iter().copied().filter(|&r| r != 0).collect();
    assert!(inked.len() >= 4, "code 65 is drawn");
    assert!(
        inked.iter().all(|&r| r == inked[0]),
        "font-3 code 65 is a solid block, not a letter: {:02X?}",
        a.rows,
    );
    // The solid block at code 54 is the whole cell, which no text font has.
    assert_eq!(font.glyph(54).expect("code 54").rows, vec![0xFF; 8], "code 54 is the full cell");
}

/// Shogun and Zork Zero ship no font at all — they take the system topaz.
///
/// Asserted so the loader's "no font on this medium" path stays exercised against a
/// real disk rather than only against an empty `Vec`.
#[test]
fn shogun_and_zork_zero_carry_no_font() {
    for disk in ["James Clavell's Shogun.adf", "Zork Zero - The Revenge of Megaboz.adf"] {
        let path = stories_dir().join(disk);
        if !path.is_file() {
            eprintln!("SKIP: gitignored floppy absent: {disk}");
            continue;
        }
        let files: Vec<(String, Vec<u8>)> = app::assets::files(&path)
            .into_iter()
            .filter(|f| f.is_on_medium())
            .filter_map(|f| {
                let n = f.name.clone();
                f.into_bytes().map(|b| (n, b))
            })
            .collect();
        let got =
            blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
        assert!(got.is_none(), "{disk} carries no font, but one parsed");
    }
}
