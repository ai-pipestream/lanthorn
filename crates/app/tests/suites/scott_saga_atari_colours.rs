//! SQ-1528: the Atari 8-bit S.A.G.A. colours, checked against the **real
//! machine** rather than against the hardware model that produces them.
//!
//! [`scott::saga_pictures::atari_colour`] turns a stored colour byte into RGB
//! through a phase model of the GTIA's fifteen hues, and until these captures
//! its two free numbers had only ever been checked against the hue NAMES in
//! the hardware manual — internal measurement cannot tell a plausible palette
//! from the right one. `machine-screenshots/atari-*.png` are the retail disks
//! running under emulation, committed to this repository; this suite decodes
//! the same records with the production readers, lays each over its frame,
//! and asks the frame what colour every stored byte really is.
//!
//! # The frames, and how each was reached
//!
//! | frame | record | colour bytes | what it is |
//! |---|---|---|---|
//! | `atari-sorcerer-title.png` | *Claymorgue* side B, file offset `0x4797` | `98 BB DE 00` | the S.A.G.A. 13 title card, before any input |
//! | `atari-sorcerer-game.png` | *Claymorgue* side B, `0x57BA` | `C4 97 F6 00` | room 1, the castle exterior, first move |
//! | `atari-scott-colorbars.png` | *Claymorgue* side B, `0x5580` | `32 87 E8 C6` | the boot "Adjust TV" card: RED, YELLOW, BLUE bars on a GREEN screen |
//! | `atari-scott-adventurland-title-flicker{1,2}.png` | *Adventureland* side B, line-art room index 99 | (the line-art palettes) | the title card, both phases of its frame flicker |
//! | `atari-scott-colorbars-flicker-{1,2}.png` | *Adventureland* side B, `0xC146` | (the line-art palettes) | its six-bar "Adjust TV" card, both phases |
//!
//! Neither colour-bar card is in its release's picture table; both were found
//! by scanning the side for a record that decodes to vertical bars, exactly
//! as the *Hulk*'s `B01250R` was on the Commodore 64 — and *Claymorgue*'s has
//! the same shape as that one (29 columns x 64 pairs at canvas x 16). Its
//! green screen is the record's FOURTH colour byte, the background register
//! §8.3 says is "never used": on this card it plainly is, which is out of
//! scope here and noted rather than fixed.
//!
//! **The line-art frames are single phases of a two-frame flicker.** A
//! line-art pixel is a pair `(a, b)` shown on alternate frames through
//! [`PALETTE_A`] and [`PALETTE_B`]; a capture holds one phase, so each frame
//! settles one palette's bytes, and the pair the two frames blend to is what
//! [`pair_rgb`] draws. Which phase a frame is was read off the frame: pair
//! value 10 (`(2, 2)`) is blue on the `$A000` phase (`0x84`) and green on the
//! `$9000` one (`0xB4`), and every case here re-checks that reading.
//!
//! # Where the picture sits in the frame
//!
//! The captures are whole emulator windows at varying sizes, with the text
//! area below the picture, so nothing is assumed about scale: the picture is
//! the bounding box of the frame's SATURATED pixels (text is grey, the picture
//! is not), resized to the decoder's own canvas, and each canvas pixel is
//! sampled at the centre of its frame block. Only pixels whose 3x3 canvas
//! neighbourhood is one stored value are read, so a fill's dither and a
//! line's edge never land in a bucket, and each bucket's colour is its
//! per-channel median. Naive fractional-coordinate guessing without the
//! alignment step produced garbage samples; this is the step.
//!
//! # Tolerance
//!
//! **48 summed over the three channels** — sixteen per channel. The frames are
//! an emulator's rendering through its own decoder matrix, not ours, so
//! bit-exactness is not on offer; the fitted model's worst miss over the
//! fifteen bytes is 39. The constants it replaced missed the three regions
//! the quest reported by 60 (the castle: olive, not gold-brown), 255 (the
//! globe: rose, not red-brown) and 106 (the banner: orange, not olive).
//!
//! # Skipping
//!
//! The frames are committed; the `.atr` sides are gitignored commercial
//! fixtures (§10.7), so every case here skips vacuously without them — and
//! says so, because a silent skip reads exactly like a pass.

use std::collections::BTreeMap;
use std::path::PathBuf;

use scott::saga_atari::{
    decode_record, draw_line_art_record, read_line_art_table, record_at, splice_vtoc, spliced_of,
};
use scott::saga_atari_lineart::{pair_rgb, LineArtCanvas, PALETTE_A, PALETTE_B};
use scott::saga_pictures::{atari_colour, FamilyCScheme, Rgb};
use scott::saga_us::PictureUsage;

use crate::fixture_paths::fixture_path;

const CLAYMORGUE_SIDE_B: &str =
    "scott-dialects/atari/SAGA No. 13 - The Sorcerer of Claymorgue Castle _ side B.atr";
const ADVENTURELAND_SIDE_B: &str = "scott-dialects/atari/SAGA #1 - Adventureland [side B].atr";

/// The tolerance the module header justifies.
const TOLERANCE: u32 = 48;

fn skipped(what: &str) -> bool {
    eprintln!("SKIP: {what} — needs the gitignored Atari 8-bit S.A.G.A. sides (§10.7)");
    true
}

fn screenshot(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../machine-screenshots").join(name)
}

/// A side with its volume table of contents excised, the way every reader
/// here wants it — or `None` when the fixture is absent.
fn side(name: &str) -> Option<Vec<u8>> {
    let raw = std::fs::read(fixture_path(name)).ok()?;
    Some(splice_vtoc(&raw))
}

/// A decoded picture as the decoder's own pixel VALUES, not colours.
struct ValueMap {
    w: usize,
    h: usize,
    values: Vec<u8>,
}

impl ValueMap {
    fn at(&self, x: usize, y: usize) -> u8 {
        self.values[y * self.w + x]
    }
}

/// The family-C record at `file_offset`, checked to carry `colour_bytes` so a
/// re-mastered side cannot quietly hand this suite a different picture.
fn bitmap(spliced: &[u8], file_offset: usize, colour_bytes: [u8; 4]) -> ValueMap {
    let record = record_at(spliced, spliced_of(file_offset), FamilyCScheme::Standard)
        .unwrap_or_else(|| panic!("a record at file offset 0x{file_offset:X}"));
    assert_eq!(
        record.colour_bytes(),
        colour_bytes,
        "the record at 0x{file_offset:X} is the one the capture shows"
    );
    let pic = decode_record(spliced, &record, FamilyCScheme::Standard).expect("decodes");
    ValueMap { w: pic.width(), h: pic.height(), values: pic.pixels().to_vec() }
}

/// The line-art record at `file_offset`, played on a fresh canvas.
fn line_art(spliced: &[u8], file_offset: usize) -> ValueMap {
    let mut canvas = LineArtCanvas::new();
    let pic = draw_line_art_record(&mut canvas, spliced, file_offset)
        .unwrap_or_else(|e| panic!("the record at 0x{file_offset:X} plays: {e}"));
    ValueMap { w: pic.width(), h: pic.height(), values: pic.pixels().to_vec() }
}

fn l1(a: Rgb, b: Rgb) -> u32 {
    u32::from(a.0.abs_diff(b.0)) + u32::from(a.1.abs_diff(b.1)) + u32::from(a.2.abs_diff(b.2))
}

fn saturated(p: [u8; 3]) -> bool {
    let hi = p[0].max(p[1]).max(p[2]);
    let lo = p[0].min(p[1]).min(p[2]);
    hi - lo > 40 && hi > 30
}

/// Per-channel median of a bucket, averaging the two middles of an even count
/// exactly as a numeric library would.
fn median(samples: &[Rgb]) -> Rgb {
    let channel = |pick: fn(&Rgb) -> u8| {
        let mut c: Vec<u8> = samples.iter().map(pick).collect();
        c.sort_unstable();
        let n = c.len();
        if n % 2 == 1 {
            c[n / 2]
        } else {
            ((u16::from(c[n / 2 - 1]) + u16::from(c[n / 2])) / 2) as u8
        }
    };
    (channel(|p| p.0), channel(|p| p.1), channel(|p| p.2))
}

/// The module header's alignment: for every stored value, the median colour
/// the frame shows at the centre of every canvas pixel whose 3x3 neighbourhood
/// is that value, with how many such pixels there were.
fn frame_medians(png: &str, map: &ValueMap) -> BTreeMap<u8, (Rgb, usize)> {
    let img = image::open(screenshot(png))
        .unwrap_or_else(|e| panic!("{png} opens: {e}"))
        .to_rgb8();
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if saturated(p.0) {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    assert!(x1 > x0 && y1 > y0, "{png}: no coloured pixels at all");
    let sx = f64::from(x1 - x0) / map.w as f64;
    let sy = f64::from(y1 - y0) / map.h as f64;
    let mut buckets: BTreeMap<u8, Vec<Rgb>> = BTreeMap::new();
    for y in 1..map.h - 1 {
        for x in 1..map.w - 1 {
            let v = map.at(x, y);
            let uniform = (y - 1..=y + 1).all(|yy| (x - 1..=x + 1).all(|xx| map.at(xx, yy) == v));
            if !uniform {
                continue;
            }
            let px = (f64::from(x0) + (x as f64 + 0.5) * sx) as u32;
            let py = (f64::from(y0) + (y as f64 + 0.5) * sy) as u32;
            let p = img.get_pixel(px.min(img.width() - 1), py.min(img.height() - 1)).0;
            buckets.entry(v).or_default().push((p[0], p[1], p[2]));
        }
    }
    buckets.into_iter().map(|(v, s)| (v, (median(&s), s.len()))).collect()
}

fn assert_close(what: &str, machine: Rgb, ours: Rgb) {
    assert!(
        l1(machine, ours) <= TOLERANCE,
        "{what}: the machine shows {machine:?}, we resolve {ours:?} — {} summed over the channels",
        l1(machine, ours)
    );
}

// ── Claymorgue Castle: family-C bitmaps ──────────────────────────────────────

/// The title card's three text colours and room 1's sky, grass and castle:
/// six stored bytes, each read straight off its frame.
#[test]
fn claymorgue_title_and_room_colours_are_the_machines() {
    let Some(spliced) = side(CLAYMORGUE_SIDE_B) else {
        assert!(skipped("Claymorgue's title and room 1"));
        return;
    };
    let frames = [
        ("atari-sorcerer-title.png", 0x4797, [0x98, 0xBB, 0xDE, 0x00], 4),
        ("atari-sorcerer-game.png", 0x57BA, [0xC4, 0x97, 0xF6, 0x00], 400),
    ];
    for (png, offset, bytes, min_samples) in frames {
        let map = bitmap(&spliced, offset, bytes);
        let medians = frame_medians(png, &map);
        for value in 1..=3u8 {
            let (machine, n) = medians[&value];
            let byte = bytes[usize::from(value) - 1];
            assert!(
                n >= min_samples,
                "{png}: value {value} has only {n} solid pixels to read — the alignment slipped"
            );
            assert_close(&format!("{png}: value {value}, colour byte 0x{byte:02X}"), machine, atari_colour(byte));
        }
        assert_eq!(medians[&0].0, (0, 0, 0), "{png}: value 0 is the black background");
    }

    // The reported symptom, by name: the castle body is gold-brown, warmer
    // than the grass, not the olive-green the old constants made it. The
    // machine's red channel leads its green by 20 there; ours may not fall
    // below the green by more than the tolerance allows above.
    let map = bitmap(&spliced, 0x57BA, [0xC4, 0x97, 0xF6, 0x00]);
    let castle = frame_medians("atari-sorcerer-game.png", &map)[&3].0;
    assert!(castle.0 > castle.1, "the machine's castle is gold-brown: {castle:?}");
    let (r, g, _) = atari_colour(0xF6);
    assert!(r + 16 >= g, "our castle is not olive: ({r}, {g}, _)");
}

/// The boot colour-bar card names its colours — RED, YELLOW, BLUE on a GREEN
/// screen — and is the disk's own calibration reference for three bytes plus
/// the background register.
#[test]
fn claymorgue_colour_bars_settle_red_yellow_blue_and_the_green_background() {
    let Some(spliced) = side(CLAYMORGUE_SIDE_B) else {
        assert!(skipped("Claymorgue's colour-bar card"));
        return;
    };
    let bytes = [0x32, 0x87, 0xE8, 0xC6];
    let map = bitmap(&spliced, 0x5580, bytes);
    // The record is one row repeated: three runs of values 1, 3, 2 across
    // canvas x 16..=237. Read the runs off the decode rather than assuming.
    let mut runs: Vec<(u8, usize, usize)> = Vec::new();
    for x in 0..map.w {
        let v = map.at(x, 0);
        match runs.last_mut() {
            Some((lv, _, end)) if *lv == v => *end = x,
            _ => runs.push((v, x, x)),
        }
    }
    let bars: Vec<(u8, usize, usize)> = runs.iter().copied().filter(|r| r.0 != 0).collect();
    assert_eq!(
        bars.iter().map(|b| b.0).collect::<Vec<_>>(),
        [1, 3, 2],
        "three bars, RED (value 1), YELLOW (value 3), BLUE (value 2): {runs:?}"
    );
    for y in 1..128 {
        assert_eq!(&map.values[y * map.w..(y + 1) * map.w], &map.values[..map.w], "row {y} repeats row 0");
    }

    let img = image::open(screenshot("atari-scott-colorbars.png")).expect("opens").to_rgb8();
    let bg = img.get_pixel(24, 24).0;
    let background = (bg[0], bg[1], bg[2]);
    assert!(bg[1] > bg[0] + 40 && bg[1] > bg[2] + 40, "the screen is green: {background:?}");
    // The frame's edges are filtered, so no bounding box: the three bars are
    // the three most frequent saturated colours that are not the screen's
    // green, each found as its own solid rectangle and ordered left to right.
    let mut counts: BTreeMap<Rgb, usize> = BTreeMap::new();
    for p in img.pixels() {
        let c = (p[0], p[1], p[2]);
        if saturated(p.0) && l1(c, background) > 30 {
            *counts.entry(c).or_default() += 1;
        }
    }
    let mut ranked: Vec<(usize, Rgb)> = counts.iter().map(|(c, n)| (*n, *c)).collect();
    ranked.sort_unstable_by(|a, b| b.cmp(a));
    let mut found: Vec<(u32, Rgb)> = Vec::new();
    for (n, colour) in ranked.iter().take(3) {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (x, y, p) in img.enumerate_pixels() {
            if (p[0], p[1], p[2]) == *colour {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
        let area = ((x1 - x0) * (y1 - y0)) as usize;
        assert!(
            *n * 10 >= area * 9 && area > 100_000,
            "{colour:?} fills {n} of its {area}-pixel rectangle — not a solid bar"
        );
        found.push((x0, *colour));
    }
    found.sort_unstable();
    let names = ["RED", "YELLOW", "BLUE"];
    for (i, (value, _, _)) in bars.iter().copied().enumerate() {
        let byte = bytes[usize::from(value) - 1];
        assert_close(&format!("{} bar, colour byte 0x{byte:02X}", names[i]), found[i].1, atari_colour(byte));
    }
    assert_close("GREEN screen, the fourth colour byte 0xC6", background, atari_colour(0xC6));
}

// ── Adventureland: line-art pairs, one palette per flicker phase ────────────

/// Which of the two bitmaps a captured frame shows, read off the frame
/// itself: pair value 10 is blue on the `$A000` phase and green on `$9000`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    A,
    B,
}

impl Phase {
    fn palette(self) -> [u8; 4] {
        match self {
            Phase::A => PALETTE_A,
            Phase::B => PALETTE_B,
        }
    }

    /// The stored byte pair value `v` shows on this phase.
    fn byte(self, v: u8) -> u8 {
        match self {
            Phase::A => self.palette()[usize::from(v / 4)],
            Phase::B => self.palette()[usize::from(v % 4)],
        }
    }

    fn identify(medians: &BTreeMap<u8, (Rgb, usize)>, png: &str) -> Phase {
        let (shown, _) = medians[&10];
        let to_a = l1(shown, atari_colour(PALETTE_A[2]));
        let to_b = l1(shown, atari_colour(PALETTE_B[2]));
        assert!(
            to_a.abs_diff(to_b) > TOLERANCE,
            "{png}: pair value 10 shows {shown:?}, which does not tell the phases apart"
        );
        if to_a < to_b {
            Phase::A
        } else {
            Phase::B
        }
    }
}

/// One pair of captures of the same picture, one per phase: each frame
/// settles its own palette's bytes, and the two together settle the blend
/// [`pair_rgb`] draws.
fn check_flicker_pair(spliced: &[u8], map: &ValueMap, pngs: [&str; 2], what: &str) {
    let _ = spliced;
    let medians = pngs.map(|png| frame_medians(png, map));
    let phases = [Phase::identify(&medians[0], pngs[0]), Phase::identify(&medians[1], pngs[1])];
    assert_ne!(phases[0], phases[1], "{what}: the two captures are the two phases");

    let mut blended = 0usize;
    for (v, (machine, n)) in &medians[0] {
        if *v == 0 {
            continue;
        }
        let Some((other, m)) = medians[1].get(v) else { continue };
        assert!(*n >= 100 && *m >= 100, "{what}: pair value {v} has {n}/{m} solid pixels");
        for (phase, shown, png) in [(phases[0], *machine, pngs[0]), (phases[1], *other, pngs[1])] {
            let byte = phase.byte(*v);
            assert_close(&format!("{png}: pair value {v} on phase {phase:?}, colour byte 0x{byte:02X}"), shown, atari_colour(byte));
        }
        // The blend: what the eye sees across both frames, against what we draw.
        let mid = |a: u8, b: u8| ((u16::from(a) + u16::from(b)) / 2) as u8;
        let eye = (mid(machine.0, other.0), mid(machine.1, other.1), mid(machine.2, other.2));
        assert_close(&format!("{what}: pair value {v} blended across both phases"), eye, pair_rgb(v / 4, v % 4));
        blended += 1;
    }
    assert!(blended >= 4, "{what}: only {blended} pair values were on both frames");
}

/// The title card: the globe (pair value 5) is a dark red-brown across both
/// phases and the banner (pair value 11) alternates blue and olive — not the
/// pink and purple the old constants drew.
#[test]
fn adventureland_title_frames_settle_the_line_art_palettes_and_their_blend() {
    let Some(spliced) = side(ADVENTURELAND_SIDE_B) else {
        assert!(skipped("Adventureland's title card"));
        return;
    };
    let table = read_line_art_table(&spliced, 1).expect("Adventureland's own picture table");
    let offset = table.find(PictureUsage::Room, 99).expect("the title card is room index 99");
    let map = line_art(&spliced, offset);
    let pngs = [
        "atari-scott-adventurland-title-flicker1.png",
        "atari-scott-adventurland-title-flicker2.png",
    ];
    check_flicker_pair(&spliced, &map, pngs, "the title card");

    // The two reported symptoms, by name.
    let globe = pair_rgb(1, 1);
    assert!(
        globe.0 > globe.1 + 40 && globe.0 > globe.2 + 40 && globe.0 < 140,
        "the globe is a dark red-brown, not pink: {globe:?}"
    );
    let banner = pair_rgb(2, 3);
    assert!(banner.2 > banner.0 && banner.1 > banner.0, "the banner is a teal-grey, not purple: {banner:?}");
}

/// The six-bar "Adjust TV" card, found by scanning side B for a record that
/// decodes to bars: WHITE, BROWN, ORANGE, YELLOW, BLUE, GREEN, drawn from the
/// same six bytes as every other line-art picture — but as solid blocks of
/// thousands of pixels each, which is the cleanest read of them there is.
#[test]
fn adventureland_colour_bars_settle_the_same_six_bytes_from_solid_blocks() {
    let Some(spliced) = side(ADVENTURELAND_SIDE_B) else {
        assert!(skipped("Adventureland's colour-bar card"));
        return;
    };
    let map = line_art(&spliced, 0xC146);
    // Non-vacuity: six bars, the second a dither the interior mask skips.
    let solid: Vec<u8> = (0..map.w).filter(|&x| (0..map.h).all(|y| map.at(x, y) == map.at(x, 0))).map(|x| map.at(x, 0)).collect();
    let mut bars = Vec::new();
    for v in solid {
        if bars.last() != Some(&v) {
            bars.push(v);
        }
    }
    assert_eq!(
        bars,
        [11, 0, 5, 0, 15, 0, 10, 0, 14],
        "WHITE, (BROWN dither), ORANGE, YELLOW, BLUE, GREEN, black between: {bars:?}"
    );
    let pngs = ["atari-scott-colorbars-flicker-1.png", "atari-scott-colorbars-flicker-2.png"];
    check_flicker_pair(&spliced, &map, pngs, "the colour-bar card");
}
