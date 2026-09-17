//! SQ-1530: a `garglk.ini`/`<stem>.ini` sidecar shipped INSIDE a zip, beside
//! the story it configures, was never found.
//!
//! `garglk_ini::discover` used to be pure-filesystem: `story_path.parent()` +
//! `<stem>.ini` + `std::fs::read_to_string`. A story loaded out of a `.zip`
//! never touches disk as a separate file — `hints::load_story_bytes` reads the
//! entry straight into memory — so `story_path` for a zipped launch is the
//! ZIP's own path, and `discover` went looking for `Kerkerkruip.ini` beside
//! the *zip* on disk, where the real sidecar never is: it is an entry inside
//! the archive.
//!
//! Reported as a colour bug: Kerkerkruip's hyperlinks are dark teal `#133939`
//! per the game's own shipped `Kerkerkruip.ini` (`linkcolor 133939`) when the
//! two files sit loose beside each other, and plain ANSI cyan — the
//! un-imported default — when the very same pair sits together inside one
//! `.zip`.
//!
//! Two halves, same shape as `zip_story_entries.rs`: hand-built zips pin the
//! MECHANISM (stem priority, the `garglk.ini` fallback, and — for a
//! multi-story archive — matching the LAUNCHED entry's own stem rather than
//! the zip's or the first `.ini` found) without needing the commercial fixture;
//! a real specimen built from `stories/Kerkerkruip.gblorb` +
//! `stories/Kerkerkruip.ini` (both gitignored, skips vacuously) pins the exact
//! reported value.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use ratatui::style::Color;

use crate::fixture_paths::fixture_path;

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// A minimal-but-coherent v5 story image — every clause
/// `blorb::adf::looks_like_zcode` checks is satisfied, because that is what
/// `hints::extract_story` (and so `hints::zipped_stories`) classifies a zip
/// entry by. Copied from `zip_story_entries.rs`'s `zcode_v5`.
fn zcode_v5(release: u16, serial: &[u8; 6]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 5;
    b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
    b[0x04..0x06].copy_from_slice(&96u16.to_be_bytes()); // high memory
    b[0x08..0x0A].copy_from_slice(&100u16.to_be_bytes()); // dictionary (static)
    b[0x0A..0x0C].copy_from_slice(&64u16.to_be_bytes()); // objects (dynamic)
    b[0x0C..0x0E].copy_from_slice(&70u16.to_be_bytes()); // globals (dynamic)
    b[0x0E..0x10].copy_from_slice(&96u16.to_be_bytes()); // static memory base
    b[0x12..0x18].copy_from_slice(serial);
    b[0x1A..0x1C].copy_from_slice(&(128u16 / 4).to_be_bytes()); // file length / 4
    b
}

/// Write a zip at `path` holding each `(entry name, bytes)` in archive order,
/// STORED so what comes back out is byte-for-byte what went in.
fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).expect("a scratch zip");
    let mut zw = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        zw.start_file(*name, opts).unwrap();
        zw.write_all(bytes).unwrap();
    }
    zw.finish().unwrap();
}

/// A scratch directory unique per CALL, not merely per process (SQ-1131).
fn scratch(tag: &str) -> PathBuf {
    app::scratch_dir(tag)
}

// ── Synthetic mechanism cases (no gitignored fixture needed; run on CI) ──────

/// A single-story zip bundling its game and its `<stem>.ini` sidecar — the
/// exact shape of the reported bug, minus the real game.
#[test]
fn single_story_zip_finds_stem_ini_sidecar() {
    let dir = scratch("garglk-zip-stem");
    let zip_path = dir.join("Foo.zip");
    let ini = b"linkcolor 445566\n";
    write_zip(&zip_path, &[("Foo.z5", &zcode_v5(1, b"010101")), ("Foo.ini", ini)]);

    let ov = app::garglk_ini::discover_with_entry(&zip_path, None)
        .expect("Foo.ini inside the zip must be found");
    assert_eq!(ov.linkcolor, Some(Color::Rgb(0x44, 0x55, 0x66)));
    assert!(
        ov.path.ends_with("Foo.ini"),
        "overlay path should name the ini entry, got {}",
        ov.path.display()
    );
}

/// No `<stem>.ini` in the archive → falls back to a bare `garglk.ini` entry.
#[test]
fn single_story_zip_falls_back_to_garglk_ini_entry() {
    let dir = scratch("garglk-zip-fallback");
    let zip_path = dir.join("Foo.zip");
    write_zip(
        &zip_path,
        &[("Foo.z5", &zcode_v5(1, b"010101")), ("garglk.ini", b"linkcolor 020202\n")],
    );

    let ov = app::garglk_ini::discover_with_entry(&zip_path, None)
        .expect("garglk.ini inside the zip must be found");
    assert_eq!(ov.linkcolor, Some(Color::Rgb(2, 2, 2)));
}

/// Both a stem ini and a bare `garglk.ini` present → the stem one wins, the
/// same priority the filesystem case has always had.
#[test]
fn single_story_zip_prefers_stem_ini_over_garglk_ini() {
    let dir = scratch("garglk-zip-priority");
    let zip_path = dir.join("Foo.zip");
    write_zip(
        &zip_path,
        &[
            ("Foo.z5", &zcode_v5(1, b"010101")),
            ("Foo.ini", b"linkcolor 010101\n"),
            ("garglk.ini", b"linkcolor 020202\n"),
        ],
    );

    let ov = app::garglk_ini::discover_with_entry(&zip_path, None).expect("stem sidecar found");
    assert_eq!(ov.linkcolor, Some(Color::Rgb(1, 1, 1)), "Foo.ini wins over garglk.ini");
}

/// Neither an ini beside the story's stem nor a bare `garglk.ini` in the
/// archive → nothing to import, exactly the filesystem case's empty answer.
#[test]
fn single_story_zip_with_no_ini_finds_nothing() {
    let dir = scratch("garglk-zip-none");
    let zip_path = dir.join("Foo.zip");
    write_zip(&zip_path, &[("Foo.z5", &zcode_v5(1, b"010101"))]);

    assert!(app::garglk_ini::discover_with_entry(&zip_path, None).is_none());
}

/// A zip whose section selector names the story's own filename
/// (`[Foo.z5]`) resolves it — proof the zip branch feeds `parse_for_story`
/// the STORY's name, not the zip's, for selector matching. The zip here is
/// deliberately named something else entirely.
#[test]
fn single_story_zip_selector_matches_the_story_filename_not_the_archive() {
    let dir = scratch("garglk-zip-selector");
    let zip_path = dir.join("if-archive-download-7.zip");
    let ini = b"[Foo.z5]\nlinkcolor 0a0b0c\n";
    write_zip(&zip_path, &[("Foo.z5", &zcode_v5(1, b"010101")), ("Foo.ini", ini)]);

    let ov = app::garglk_ini::discover_with_entry(&zip_path, None).expect("Foo.ini found");
    assert_eq!(ov.linkcolor, Some(Color::Rgb(0x0a, 0x0b, 0x0c)));
    assert_eq!(ov.matched, vec!["Foo.z5".to_string()]);
}

/// A zip holding TWO stories, each with its own `.ini` sidecar (SQ-1098's
/// `disk_entry` naming which one is being launched): the overlay found must
/// be the LAUNCHED entry's own, not the first ini the archive happens to list.
#[test]
fn multi_story_zip_matches_the_launched_entrys_own_stem() {
    let dir = scratch("garglk-zip-multi");
    let zip_path = dir.join("compilation.zip");
    write_zip(
        &zip_path,
        &[
            ("Amber.z5", &zcode_v5(1, b"aaaaaa")),
            ("Amber.ini", b"linkcolor 111111\n"),
            ("Beacon.z5", &zcode_v5(2, b"bbbbbb")),
            ("Beacon.ini", b"linkcolor 222222\n"),
        ],
    );

    let amber = app::garglk_ini::discover_with_entry(&zip_path, Some("Amber.z5"))
        .expect("Amber.ini found for the Amber entry");
    assert_eq!(amber.linkcolor, Some(Color::Rgb(0x11, 0x11, 0x11)));

    let beacon = app::garglk_ini::discover_with_entry(&zip_path, Some("Beacon.z5"))
        .expect("Beacon.ini found for the Beacon entry");
    assert_eq!(beacon.linkcolor, Some(Color::Rgb(0x22, 0x22, 0x22)));
}

/// A non-zip path must still take the ordinary filesystem branch —
/// `discover_with_entry` is a superset of `discover`, never a narrowing.
#[test]
fn plain_file_path_is_unaffected_by_the_zip_branch() {
    let dir = scratch("garglk-zip-not-a-zip");
    let story = dir.join("zork1.z3");
    std::fs::write(&story, b"dummy").unwrap();
    std::fs::write(dir.join("zork1.ini"), "linkcolor 030303\n").unwrap();

    let ov = app::garglk_ini::discover_with_entry(&story, None).expect("filesystem sidecar found");
    assert_eq!(ov.linkcolor, Some(Color::Rgb(3, 3, 3)));
    assert!(ov.path.is_file(), "filesystem case still returns a real, readable path");
}

// ── Real specimen (stories/ only; skips vacuously) ────────────────────────────

/// The exact reported bug: `stories/Kerkerkruip.gblorb` + the game's own
/// shipped `stories/Kerkerkruip.ini` (`linkcolor 133939`), zipped together
/// exactly as the user's second machine had them, then resolved through the
/// SAME single-story path (`disk_entry: None`) a bare `lanthorn Kerkerkruip.zip`
/// launch takes.
#[test]
fn real_kerkerkruip_zip_finds_the_dark_teal_linkcolor() {
    let gblorb_path = fixture_path("Kerkerkruip.gblorb");
    let ini_path = fixture_path("Kerkerkruip.ini");
    if !gblorb_path.is_file() || !ini_path.is_file() {
        eprintln!("SKIP: gitignored fixtures missing (Kerkerkruip.gblorb / Kerkerkruip.ini)");
        return;
    }
    let gblorb = std::fs::read(&gblorb_path).expect("read the real gblorb");
    let ini = std::fs::read(&ini_path).expect("read the real ini");

    let dir = scratch("garglk-zip-kerkerkruip");
    let zip_path = dir.join("Kerkerkruip.zip");
    write_zip(&zip_path, &[("Kerkerkruip.gblorb", &gblorb), ("Kerkerkruip.ini", &ini)]);

    let ov = app::garglk_ini::discover_with_entry(&zip_path, None)
        .expect("the shipped Kerkerkruip.ini must be found inside the zip");
    assert_eq!(
        ov.linkcolor,
        Some(Color::Rgb(0x13, 0x39, 0x39)),
        "the game's own ini sets `linkcolor 133939` — the reported dark teal"
    );
    assert_eq!(
        ov.matched,
        vec!["Kerkerkruip.gblorb".to_string()],
        "the ini's `[ Kerkerkruip.gblorb ]` section must be the one that matched"
    );
}
