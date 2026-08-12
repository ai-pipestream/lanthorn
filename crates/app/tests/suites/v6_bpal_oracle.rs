//! The Blorb `BPal` table as a ground-truth oracle for the adaptive-palette
//! path (Blorb §11.3, SQ-0720).
//!
//! Infocom's v6 blorbs carry a non-standard `BPal` chunk beside `APal`: a flat
//! `(background, adaptive, baked)` table in which the converter has already
//! performed the §11.3 computation for every (non-adaptive picture, adaptive
//! picture) pair and stored the finished picture as an extra `Pict` resource in
//! a high-id block. Zork Zero's table is 224x172 = 38528 rows; Arthur's is
//! 134x3 = 402.
//!
//! `PictSource` computes the same thing live, by splicing the Current Palette's
//! PLTE into the adaptive picture's PNG. So every row is an independent answer
//! from a party that never saw our code: draw the background (which establishes
//! the Current Palette), draw the adaptive picture, and the decoded pixels must
//! equal the converter's baked variant byte for byte. This test replays the
//! WHOLE table for both blorbs — 38930 rows, ~3s in a debug build.
//!
//! Falsification (SQ-0720, both re-checked by hand before landing this):
//!  * skip the background draw so no Current Palette is established and every
//!    one of the 38930 rows mismatches — the oracle is not vacuous;
//!  * make `splice_plte` zero-fill instead of keeping the placeholder's trailing
//!    entries when the Current Palette is SHORTER, and 131 Zork0 rows across 14
//!    backgrounds break — exactly the 131 rows `short_palette_rows` counts.
//!    Those rows are the whole reason it is asserted below: they are the only
//!    coverage that branch has, and the converter independently agrees with the
//!    rule we chose. (This is also the quest's reported outlier, `background=503
//!    adaptive=18 baked=1520`: Pict 18 paints 44 pixels with index 7, Pict 503's
//!    palette stops at index 6, and the converter — like us — coloured them from
//!    the placeholder's own entry 7.)
//!
//! Skips vacuously when the gitignored blorbs are absent (CI).

use std::collections::HashMap;
use std::path::PathBuf;

use app::graphics::PictSource;
use blorb::bpal::PaletteBake;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// `(width, height, RGBA8 bytes)` of a decoded picture.
fn rgba(img: &image::DynamicImage) -> (u32, u32, Vec<u8>) {
    let b = img.to_rgba8();
    (b.width(), b.height(), b.into_raw())
}

/// Number of entries in a PNG's `PLTE` chunk, or `None` for a non-indexed
/// picture. Deliberately re-derived here rather than reused from `graphics.rs`:
/// an oracle that shares the implementation's palette reader would share its
/// mistakes too.
fn plte_entries(png: &[u8]) -> Option<usize> {
    if png.len() < 8 || &png[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let mut q = 8;
    while q + 12 <= png.len() {
        let len = u32::from_be_bytes([png[q], png[q + 1], png[q + 2], png[q + 3]]) as usize;
        if q + 12 + len > png.len() {
            return None;
        }
        if &png[q + 4..q + 8] == b"PLTE" {
            return Some(len / 3);
        }
        q += 12 + len;
    }
    None
}

struct Report {
    rows: usize,
    exact: usize,
    /// Rows whose background palette has FEWER entries than the adaptive
    /// picture's placeholder — the `splice_plte` "keep the trailing entries"
    /// branch. Zero here would mean that branch lost its only coverage.
    short_palette_rows: usize,
    /// Human-readable detail for the first handful of failures.
    failures: Vec<String>,
}

/// Replay every `BPal` row of `file`, or `None` when the gitignored blorb is
/// absent.
fn replay(file: &str) -> Option<Report> {
    let path = stories_dir().join(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored blorb missing at {}", path.display());
        return None;
    };
    let table = blorb::bpal::palette_bakes(&bytes);
    assert!(!table.is_empty(), "{file} must carry a BPal table");

    let blorb = blorb::Blorb::parse(bytes.clone()).expect("valid blorb");
    // Cross-check the two chunks describe the same thing before trusting either:
    // every `adaptive` column value is APal-listed, and no `background` one is.
    for r in &table {
        assert!(
            blorb.is_adaptive_picture(r.adaptive),
            "{file}: BPal column 2 ({}) must be an APal picture",
            r.adaptive
        );
        assert!(
            !blorb.is_adaptive_picture(r.background),
            "{file}: BPal column 1 ({}) must NOT be an APal picture",
            r.background
        );
    }

    // Palette entry counts, straight off the raw PNGs (not via PictSource).
    let entries = |n: u32| -> Option<usize> {
        blorb.resource(b"Pict", n).and_then(|(_ty, png)| plte_entries(png))
    };

    // The expected side: each baked variant decoded from its own bytes, with no
    // PictSource involved at all, memoised because ~1710 variants cover 38528
    // rows.
    let mut baked: HashMap<u32, (u32, u32, Vec<u8>)> = HashMap::new();

    // The table is grouped by background; walk it in runs so one `PictSource`
    // serves a whole background (a fresh one per run keeps the adaptive decode
    // cache from growing to 38528 images).
    let mut report =
        Report { rows: table.len(), exact: 0, short_palette_rows: 0, failures: Vec::new() };
    let mut run: Vec<PaletteBake> = Vec::new();
    let mut flush = |run: &mut Vec<PaletteBake>, report: &mut Report| {
        let Some(&first) = run.first() else { return };
        let mut picts = PictSource::new(Some(blorb::Blorb::parse(bytes.clone()).expect("blorb")));
        // Drawing the non-adaptive background is what establishes the Current
        // Palette (§11.3) — the whole precondition the table describes.
        assert!(
            picts.image(first.background).is_some(),
            "{file}: background Pict {} must decode",
            first.background
        );
        let bg_entries = entries(first.background);
        for r in run.drain(..) {
            if let (Some(bg), Some(ad)) = (bg_entries, entries(r.adaptive)) {
                if bg < ad {
                    report.short_palette_rows += 1;
                }
            }
            let got = picts.image(r.adaptive).expect("adaptive Pict decodes");
            let want = baked.entry(r.baked).or_insert_with(|| {
                let (_ty, png) = blorb.resource(b"Pict", r.baked).expect("baked Pict exists");
                rgba(&app::cover::decode(png).expect("baked Pict decodes"))
            });
            let got = rgba(&got);
            if got == *want {
                report.exact += 1;
            } else if report.failures.len() < 10 {
                let diff = if (got.0, got.1) != (want.0, want.1) {
                    format!("size {}x{} vs {}x{}", got.0, got.1, want.0, want.1)
                } else {
                    let px = got
                        .2
                        .chunks_exact(4)
                        .zip(want.2.chunks_exact(4))
                        .filter(|(a, b)| a != b)
                        .count();
                    format!("{px} of {} pixels differ", got.0 * got.1)
                };
                report.failures.push(format!(
                    "background={} adaptive={} baked={}: {diff}",
                    r.background, r.adaptive, r.baked
                ));
            }
        }
    };
    for r in &table {
        if run.first().is_some_and(|f| f.background != r.background) {
            flush(&mut run, &mut report);
        }
        run.push(*r);
    }
    flush(&mut run, &mut report);
    Some(report)
}

fn assert_table_reproduced(file: &str, min_rows: usize, min_short_palette_rows: usize) {
    let Some(r) = replay(file) else { return };
    eprintln!(
        "{file}: {}/{} BPal rows reproduced exactly ({} through a short Current Palette)",
        r.exact, r.rows, r.short_palette_rows
    );
    assert!(r.rows >= min_rows, "{file}: only {} BPal rows — wrong fixture?", r.rows);
    assert_eq!(
        r.exact,
        r.rows,
        "{file}: {} of {} BPal rows disagree with the converter's bake:\n  {}",
        r.rows - r.exact,
        r.rows,
        r.failures.join("\n  ")
    );
    assert!(
        r.short_palette_rows >= min_short_palette_rows,
        "{file}: only {} rows exercise a Current Palette shorter than the adaptive \
         placeholder — the splice's trailing-entry branch has lost its coverage",
        r.short_palette_rows
    );
}

/// Zork Zero: 224 backgrounds x 172 adaptive overlays. 131 of those rows pair a
/// background with an overlay whose placeholder palette has MORE entries — and
/// they are exactly the 131 the zero-fill falsification broke — so the run also
/// pins `splice_plte`'s trailing-entry rule against the converter.
#[test]
fn zork0_bpal_table_matches_our_adaptive_decode() {
    assert_table_reproduced("Zork0.blb", 38_000, 131);
}

/// Arthur: a second, much smaller specimen (134 backgrounds x 3 overlays), whose
/// palettes are all full-length — it covers the plain splice, not the short one.
#[test]
fn arthur_bpal_table_matches_our_adaptive_decode() {
    assert_table_reproduced("Arthur.blb", 400, 0);
}
