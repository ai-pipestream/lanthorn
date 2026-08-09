//! SQ-0719: the Amiga palette, and the fact that selecting it changes nothing
//! until it is selected.
//!
//! Its own test binary on purpose. `zvm::screen::set_palette` is process-wide
//! (see its docs for why), so a test that flips it would perturb any test
//! sharing its binary; here there is nothing else to perturb, and the flip is
//! confined to one `#[test]` that puts it back.
//!
//! The values are transcribed from Infocom's own Amiga Version 6 interpreter
//! source — `colortable[]`/`colormap[]` in `amiga/yzip1.c` and `DEF_FORE`/
//! `DEF_BACK` in `amiga/yzip.h` — not from any modern reconstruction. The
//! interpreter number is ZMSD §11.1.3.

use zvm::screen::{
    amiga_true_colour, grey_rgb, palette, rgb15_to_888, set_palette, standard_true_colour,
    zmsd_true_colour, Palette,
};

/// Widen an Amiga 4-bit-per-channel `$0RGB` word to the Z-machine's 15-bit
/// `0bbbbbgggggrrrrr`, by bit replication — the derivation the constants in
/// `amiga_true_colour` were computed with, written out independently here so a
/// typo in the table cannot agree with itself.
fn amiga4(rgb444: u16) -> u16 {
    let w = |n: u16| (n << 1) | (n >> 3);
    let (r, g, b) = ((rgb444 >> 8) & 0xF, (rgb444 >> 4) & 0xF, rgb444 & 0xF);
    (w(b) << 10) | (w(g) << 5) | w(r)
}

#[test]
fn the_amiga_table_is_infocoms_own_colortable() {
    // Left column: the Z-machine colour number. Right: the raw `$0RGB` word the
    // Amiga interpreter passed to SetRGB4 for it, read through `colormap[]`.
    //
    //   colormap[] = { -1, -1, 2, 4, 3, 5, 0, 6, 7, 1, 8, 9, 10 }
    //   colortable[] = { $005A, $0FFF, $0000, $00C0, $0E00, $0EE0,
    //                    $0F0F, $00EE, $0AAA, $x777, $0444 }
    let expected: [(u8, u16); 11] = [
        (2, 0x000),  // colortable[2]  black
        (3, 0xE00),  // colortable[4]  red
        (4, 0x0C0),  // colortable[3]  green
        (5, 0xEE0),  // colortable[5]  yellow
        (6, 0x05A),  // colortable[0]  blue
        (7, 0xF0F),  // colortable[6]  magenta
        (8, 0x0EE),  // colortable[7]  cyan
        (9, 0xFFF),  // colortable[1]  white
        (10, 0xAAA), // colortable[8]  light grey
        (11, 0x777), // colortable[9]  medium grey
        (12, 0x444), // colortable[10] dark grey
    ];
    for (n, raw) in expected {
        assert_eq!(
            amiga_true_colour(n),
            Some(amiga4(raw)),
            "standard colour {n} = Amiga ${raw:03X}",
        );
    }
    // The sentinels and reserved numbers have no colour on any machine.
    for n in [0u8, 1, 13, 14, 15, 16, 255] {
        assert_eq!(amiga_true_colour(n), None, "colour {n} is not a colour");
    }
}

#[test]
fn the_two_palettes_agree_where_the_standard_was_read_off_an_amiga() {
    // Six of the eleven entries are bit-for-bit identical, which is the strongest
    // evidence available that ZMSD §8.3.1's "recommended" values were themselves
    // taken from an Amiga — and a useful sanity check on the transcription, since
    // an error in the derivation would be very unlikely to reproduce them.
    for n in [2u8, 3, 5, 7, 8, 9] {
        assert_eq!(
            amiga_true_colour(n),
            zmsd_true_colour(n),
            "colour {n} is the same on both",
        );
    }
    // …and genuinely differ exactly where the machines differ: green, blue, and
    // the three Version 6 greys.
    for n in [4u8, 6, 10, 11, 12] {
        assert_ne!(
            amiga_true_colour(n),
            zmsd_true_colour(n),
            "colour {n} should differ between the palettes",
        );
    }
}

/// Both halves of the switch in ONE test, deliberately: the palette is
/// process-wide, and cargo runs a binary's tests on parallel threads, so a
/// "before" test and an "after" test would race each other rather than describe
/// a sequence.
#[test]
fn the_palette_defaults_to_standard_and_moves_every_resolver_together() {
    // The acceptance criterion in miniature: adding a second palette must not
    // move the first. Nothing has selected a palette, so this is what every
    // existing session sees.
    assert_eq!(palette(), Palette::Standard);
    for n in 0u8..=16 {
        assert_eq!(standard_true_colour(n), zmsd_true_colour(n), "colour {n}");
    }
    assert_eq!(grey_rgb(10), rgb15_to_888(0x5AD6), "light grey stays §8.3.1's");
    assert_eq!(grey_rgb(11), rgb15_to_888(0x4631), "medium grey stays §8.3.1's");
    assert_eq!(grey_rgb(12), rgb15_to_888(0x2D6B), "dark grey stays §8.3.1's");

    // One switch, or a game colour would look like two different colours on the
    // same screen: `standard_true_colour` feeds the game's own window properties
    // 17/18, `grey_rgb` feeds both renderers' Version 6 greys.
    set_palette(Palette::Amiga);
    assert_eq!(palette(), Palette::Amiga);
    for n in 0u8..=16 {
        assert_eq!(standard_true_colour(n), amiga_true_colour(n), "colour {n}");
    }
    for n in [10u8, 11, 12] {
        assert_eq!(
            grey_rgb(n),
            rgb15_to_888(amiga_true_colour(n).expect("a grey")),
            "grey {n} follows the palette",
        );
    }
    // Out-of-range still reads as dark grey, exactly as it always has.
    assert_eq!(grey_rgb(2), rgb15_to_888(amiga_true_colour(12).expect("dark grey")));

    set_palette(Palette::Standard);
    assert_eq!(palette(), Palette::Standard, "restored for anything that follows");
}
