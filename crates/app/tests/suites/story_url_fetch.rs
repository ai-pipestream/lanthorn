//! SQ-1086 — a URL is accepted wherever a story path is.
//!
//! The unit tests in `app::story_url` cover recognition, filename derivation and
//! sanitisation. What only an integration suite can settle is the claim the
//! feature rests on: **the fetch hands its file to the ordinary loader, so every
//! filetype lanthorn already opens works without a second loader.** So these
//! cases fetch through a canned [`UrlSource`] — nothing here ever touches the
//! network — and then ask the real `hints::load_mounted_story` and the real
//! `picker::scan_stories` whether the result is a story and whether the library
//! can see it.
//!
//! Two of them read `stories/`, which is gitignored, and skip vacuously when it
//! is absent (CI has no such directory). The rest build their payloads in memory
//! and always run.

use std::path::{Path, PathBuf};

use app::story_url::{
    fetch_to_dir, keep_in_library, FetchError, KeepMode, Payload, UrlSource,
};

// ── Harness ──────────────────────────────────────────────────────────────────

/// A [`UrlSource`] that hands back bytes already in hand. The whole network seam.
struct Canned {
    bytes: Vec<u8>,
    disposition: Option<String>,
}
impl UrlSource for Canned {
    fn get(&self, _url: &str) -> Result<Payload, FetchError> {
        Ok(Payload { disposition: self.disposition.clone(), bytes: self.bytes.clone() })
    }
}

fn canned(bytes: Vec<u8>) -> Canned {
    Canned { bytes, disposition: None }
}

/// A fresh scratch directory, named for the case so parallel cases cannot meet.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lanthorn-sq1086-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// The gitignored library of commercial fixtures, or `None` — every case that
/// reads it skips vacuously so CI stays green (see `CLAUDE.md`, Test fixtures).
fn stories_dir() -> Option<PathBuf> {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stories");
    d.is_dir().then_some(d)
}

/// A minimal-but-coherent v5 story image. Every clause of
/// `blorb::adf::looks_like_zcode` is satisfied, because that is the predicate
/// both the fetch's content sniff and the real loader apply (SQ-0889).
fn zcode_v5() -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 5;
    b[0x04..0x06].copy_from_slice(&96u16.to_be_bytes()); // high memory
    b[0x08..0x0A].copy_from_slice(&100u16.to_be_bytes()); // dictionary (static)
    b[0x0A..0x0C].copy_from_slice(&64u16.to_be_bytes()); // objects (dynamic)
    b[0x0C..0x0E].copy_from_slice(&70u16.to_be_bytes()); // globals (dynamic)
    b[0x0E..0x10].copy_from_slice(&96u16.to_be_bytes()); // static memory base
    b[0x12..0x18].copy_from_slice(b"890714"); // serial
    b[0x1A..0x1C].copy_from_slice(&(128u16 / 4).to_be_bytes()); // file length / 4
    b
}

/// A real ZIP holding one `.z5` entry — the shape `hints::read_story_file`
/// unwraps. Built rather than committed so the case never depends on a fixture.
fn zip_of_a_story() -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file(
            "curses.z5",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .expect("zip entry");
        zip.write_all(&zcode_v5()).expect("zip write");
        zip.finish().expect("zip finish");
    }
    buf.into_inner()
}

// ── The loader hand-off ──────────────────────────────────────────────────────

/// The whole claim of the feature, on the formats that need no fixture: fetch,
/// then hand the file to the loader lanthorn uses for a path typed by hand.
#[test]
fn a_fetched_z_code_image_opens_through_the_ordinary_loader() {
    let dir = scratch("zcode");
    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &dir)
        .expect("a coherent v5 image is fetchable");
    assert_eq!(got.path, dir.join("curses.z5"));
    assert!(
        matches!(app::hints::load_mounted_story(&got.path), Ok((app::hints::LoadedStory::ZCode(_), None))),
        "the fetched file opens as Z-code, off the same path a typed one takes"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_fetched_glulx_image_opens_through_the_ordinary_loader() {
    let dir = scratch("glulx");
    let mut glulx = b"Glul".to_vec();
    glulx.extend_from_slice(&[0u8; 60]);
    let got = fetch_to_dir(&canned(glulx), "https://example.org/if/kerkerkruip.ulx", &dir)
        .expect("a Glulx image is fetchable");
    assert!(matches!(
        app::hints::load_mounted_story(&got.path),
        Ok((app::hints::LoadedStory::Glulx(_), None))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A ZIP is not unpacked by the fetch — it is written as a `.zip` and the LOADER
/// unwraps it, exactly as it does for a zip already sitting in a library. That is
/// what "hand the file to the existing loader" buys: no second unpacker.
#[test]
fn a_fetched_zip_is_unwrapped_by_the_loader_not_by_the_fetch() {
    let dir = scratch("zip");
    let got = fetch_to_dir(&canned(zip_of_a_story()), "https://example.org/if/curses.zip", &dir)
        .expect("a zip holding a story is fetchable");
    assert_eq!(got.path.extension().unwrap(), "zip", "the archive keeps its own name");
    assert!(matches!(
        app::hints::load_mounted_story(&got.path),
        Ok((app::hints::LoadedStory::ZCode(_), None))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A download URL that names no file at all still lands under a name the library
/// can see — the extension comes from the bytes, which is the only source that
/// cannot lie about what the file is.
#[test]
fn an_extensionless_download_url_still_lands_openable() {
    let dir = scratch("noext");
    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/download.php?id=7", &dir)
        .expect("fetchable");
    assert_eq!(got.path, dir.join("download.php.z5"));
    assert!(app::hints::load_mounted_story(&got.path).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

/// "A URL that is not a story at all" must be legible, not a crash and not an
/// empty picker: the message names what arrived, and nothing is written.
#[test]
fn a_login_page_is_refused_with_a_message_naming_what_arrived() {
    let dir = scratch("login");
    let page = b"<!DOCTYPE html><html><head><title>Sign in</title></head></html>".to_vec();
    let err = fetch_to_dir(&canned(page), "https://example.org/if/curses.z5", &dir)
        .expect_err("an HTML page is not a story");
    let msg = err.to_string();
    assert!(msg.contains("a web page"), "says what it fetched: {msg}");
    assert!(msg.contains("bytes"), "and how much of it: {msg}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "and writes nothing");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Keeping it ───────────────────────────────────────────────────────────────

/// The point of the keep prompt: after a keep, the directory the PICKER scans
/// holds the story. Asserted through `picker::scan_stories` itself rather than a
/// file-exists check, because "the next launch finds it" is a claim about the
/// scan, not about the filesystem.
#[test]
fn keeping_a_fetched_story_puts_it_where_the_picker_will_find_it() {
    let temp = scratch("keep-temp");
    let library = scratch("keep-lib");
    let data_base = scratch("keep-data");

    assert!(
        app::picker::scan_stories(&library, &data_base).is_empty(),
        "the library starts empty, so the row below can only come from the keep"
    );

    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &temp)
        .expect("fetchable");
    let kept = keep_in_library(&got.path, &library, KeepMode::KeepBoth).expect("kept");
    assert_eq!(kept, library.join("curses.z5"));
    assert!(got.path.exists(), "the running game's own file survives the copy");

    let rows = app::picker::scan_stories(&library, &data_base);
    assert_eq!(rows.len(), 1, "the picker lists exactly the story that was kept");
    assert_eq!(rows[0].path, kept);

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Declining keeps the library untouched — a fetch that is not kept must leave
/// no trace in the directory the picker reads.
#[test]
fn declining_leaves_the_library_exactly_as_it_was() {
    let temp = scratch("decline-temp");
    let library = scratch("decline-lib");
    let data_base = scratch("decline-data");

    let got = fetch_to_dir(&canned(zcode_v5()), "https://example.org/if/curses.z5", &temp)
        .expect("fetchable");
    // No `keep_in_library` call is what "declined" means.
    assert!(app::picker::scan_stories(&library, &data_base).is_empty());
    assert!(got.path.exists(), "and it still plays from where it landed");

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

// ── Real media (skips vacuously without `stories/`) ──────────────────────────

/// The claim "every supported filetype works for free" against files that are
/// actually of those types — release disk images, Blorbs and bare story images
/// alike. Each one is replayed through the fetch as if it had come off the
/// network, and then opened.
///
/// A sample rather than the whole directory: the point is coverage of FORMATS,
/// and a few hundred megabytes of copies would be a slow test, not a better one.
#[test]
fn real_media_of_every_format_present_survives_a_round_trip_through_the_fetch() {
    let Some(stories) = stories_dir() else {
        eprintln!("skipping: no stories/ directory");
        return;
    };
    let dir = scratch("realmedia");

    // One specimen per extension, so the sample is a census of formats rather
    // than of filenames.
    let mut by_ext: std::collections::BTreeMap<String, (u64, PathBuf)> =
        std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(&stories).into_iter().flatten().flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        let ext = ext.to_ascii_lowercase();
        // Only formats the loader claims; anything else in the directory (a
        // README, a cover, a `.mg1`) is not a story and is not this test's
        // business.
        if !matches!(
            ext.as_str(),
            "z3" | "z4" | "z5" | "z6" | "z7" | "z8" | "ulx" | "blb" | "blorb" | "zblorb" | "gblorb"
                | "adf" | "d64" | "st" | "2mg" | "po" | "dsk" | "img" | "ima" | "image"
        ) {
            continue;
        }
        // Skip only what the fetch itself would refuse — the cap, not a number
        // of this test's own. That is deliberate: `Kerkerkruip.gblorb` is
        // 22,109,534 bytes and used to sit above BOTH, so a private limit here
        // would have quietly agreed with the defect (SQ-1086).
        if entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > app::ifdb_search::MAX_DOWNLOAD {
            continue;
        }
        // The LARGEST specimen of each format, not the first the directory
        // happens to hand back: the size cap is what this case most needs to
        // exercise, and `Kerkerkruip.gblorb` (22,109,534 bytes) is the file that
        // proves it — an arbitrary `.gblorb` of 200 KB proves nothing about it.
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        match by_ext.get(&ext) {
            Some((seen, _)) if *seen >= len => {}
            _ => {
                by_ext.insert(ext, (len, path));
            }
        }
    }
    if by_ext.is_empty() {
        eprintln!("skipping: stories/ holds no readable specimen");
        return;
    }

    // The baseline has to be the file ISOLATED, not the file in `stories/`.
    // `hints::load_mounted_story` mounts a disk image with the other volumes of
    // its release beside it (SQ-0864), so `zork_zero_4.dsk` opens in a library
    // and holds no story at all on its own — and a fetch of one volume delivers
    // exactly that, one volume. Refusing it (legibly, having removed the file it
    // wrote) is the right answer, not a defect, so such a specimen is not this
    // case's business; probe each candidate alone and sweep only the ones that
    // are self-contained.
    let probe = scratch("realmedia-probe");
    let mut opened = 0usize;
    for (ext, (len, path)) in &by_ext {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("story");
        let alone = probe.join(name);
        if std::fs::write(&alone, &bytes).is_err() {
            continue;
        }
        let self_contained = app::hints::load_mounted_story(&alone).is_ok();
        let _ = std::fs::remove_file(&alone);
        if !self_contained {
            continue;
        }
        let url = format!("https://example.org/if/{name}");
        let got = fetch_to_dir(&canned(bytes), &url, &dir)
            .unwrap_or_else(|e| panic!("{ext}: {} did not survive a fetch: {e}", path.display()));
        assert!(
            app::hints::load_mounted_story(&got.path).is_ok(),
            "{ext}: fetched {} does not open, though the original does",
            path.display()
        );
        opened += 1;
        // Name the specimen, as any fixture-driven case here does — the sizes
        // are the whole point of the cap this exercises.
        eprintln!("  {ext}: {} ({len} bytes)", path.display());
    }
    assert!(opened > 0, "non-vacuity: at least one real specimen must have been exercised");
    eprintln!("round-tripped {opened} specimens of {} candidate formats", by_ext.len());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&probe);
}

// ── The cap is a measurement, not an impression (SQ-1086) ────────────────────

/// `MAX_DOWNLOAD` used to be 16 MiB under a comment asserting that "a real
/// Z-code/Glulx/blorb rarely exceeds a few MiB", which refused *Kerkerkruip*.
/// This case is what stops the number drifting back into an opinion: it reads
/// the corpus off the disk and checks the constant against it.
///
/// The cap-versus-constant half needs no test at all: `ifdb_search` carries a
/// `const _: () = assert!(MAX_DOWNLOAD > corpus::LARGEST_GAME)`, so lowering
/// the cap is a BUILD failure rather than a test failure. What only a test can
/// check is the constant against the corpus it claims to describe — and that
/// needs the gitignored `stories/`, so it skips vacuously.
#[test]
fn the_download_cap_admits_every_game_in_the_corpus() {
    let Some(stories) = stories_dir() else {
        eprintln!("skipping the corpus half: no stories/ directory");
        return;
    };
    let mut largest: (u64, String) = (0, String::new());
    for entry in std::fs::read_dir(&stories).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
        if !matches!(
            ext.to_ascii_lowercase().as_str(),
            "z3" | "z4" | "z5" | "z6" | "z7" | "z8" | "ulx" | "blb" | "blorb" | "zblorb" | "gblorb"
        ) {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if len > largest.0 {
            largest = (len, path.display().to_string());
        }
    }
    if largest.0 == 0 {
        eprintln!("skipping the corpus half: no bare story files present");
        return;
    }
    assert!(
        app::ifdb_search::MAX_DOWNLOAD > largest.0,
        "the cap ({}) refuses {} ({} bytes) — raise it, and raise corpus::LARGEST_GAME with it",
        app::ifdb_search::MAX_DOWNLOAD,
        largest.1,
        largest.0,
    );
    assert!(
        app::corpus::LARGEST_GAME >= largest.0,
        "corpus::LARGEST_GAME ({}) is stale: {} is {} bytes",
        app::corpus::LARGEST_GAME,
        largest.1,
        largest.0,
    );
}

// ── A kept archive is visible (SQ-1086) ─────────────────────────────────────

/// The defect this pair exists for: the keep prompt says "it will be there next
/// time", and before `picker::has_story_ext` admitted `.zip` that was false for
/// an archive — the file landed in the library and the only view that would show
/// it did not list it.
#[test]
fn a_kept_zip_is_visible_to_the_picker() {
    let temp = scratch("zipkeep-temp");
    let library = scratch("zipkeep-lib");
    let data_base = scratch("zipkeep-data");

    let got = fetch_to_dir(&canned(zip_of_a_story()), "https://example.org/if/curses.zip", &temp)
        .expect("a zip holding a story is fetchable");
    let kept = keep_in_library(&got.path, &library, KeepMode::KeepBoth).expect("kept");
    assert_eq!(kept.extension().unwrap(), "zip");

    let rows = app::picker::scan_stories(&library, &data_base);
    assert_eq!(rows.len(), 1, "the archive the player kept is listed");
    assert_eq!(rows[0].path, kept);

    for d in [temp, library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// …and admitting `.zip` must not turn every archive into a row. The scan opens
/// each candidate and drops it unless a story comes out, so an archive of
/// something else appears nowhere — the property that makes a generic extension
/// safe to admit at all.
#[test]
fn an_archive_holding_no_story_is_not_listed() {
    use std::io::Write;
    let library = scratch("zipjunk-lib");
    let data_base = scratch("zipjunk-data");

    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file("holiday.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"not a story, just a note").unwrap();
        zip.finish().unwrap();
    }
    std::fs::write(library.join("photos.zip"), buf.into_inner()).unwrap();

    assert!(
        app::picker::scan_stories(&library, &data_base).is_empty(),
        "an archive with no story in it costs one open and yields no row"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// Admitting `.zip` to the scan must not turn a hint ARCHIVE into a game row.
///
/// `hints.rs` explicitly expects a hint zip beside a story (its sidecar
/// resolution scans sibling zips), and the loose `deadlineinv.z5` form has
/// always folded into the game's row rather than listing as a game of its own.
/// The archive is the same file in a wrapper and now folds the same way — the
/// alternative, observed before the fix, was `deadline-hints.zip` sitting in the
/// list as a playable title next to the very story it belongs to.
#[test]
fn a_hint_archive_folds_into_its_game_rather_than_listing_as_one() {
    let library = scratch("hintzip-lib");
    let data_base = scratch("hintzip-data");

    std::fs::write(library.join("deadline.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadline-hints.zip"), zip_of_a_story()).unwrap();

    let rows = app::picker::scan_stories(&library, &data_base);
    assert_eq!(rows.len(), 1, "one game, not a game plus its clues: {rows:?}");
    assert_eq!(rows[0].filename, "deadline.z5");
    assert_eq!(
        rows[0].hint_sidecar.as_deref(),
        Some(library.join("deadline-hints.zip").as_path()),
        "and the archive is attached as that game's hints"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

/// …and a LOOSE hint file still wins when both are present, so admitting `.zip`
/// cannot change which sidecar an existing library already resolves. An archive
/// is only ever chosen where nothing loose answered — the case that had no
/// answer at all before.
#[test]
fn a_loose_hint_file_still_outranks_an_archived_one() {
    let library = scratch("hintboth-lib");
    let data_base = scratch("hintboth-data");

    std::fs::write(library.join("deadline.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadlineinv.z5"), zcode_v5()).unwrap();
    std::fs::write(library.join("deadline-hints.zip"), zip_of_a_story()).unwrap();

    let rows = app::picker::scan_stories(&library, &data_base);
    let game = rows.iter().find(|r| r.filename == "deadline.z5").expect("the game is listed");
    assert_eq!(
        game.hint_sidecar.as_deref(),
        Some(library.join("deadlineinv.z5").as_path()),
        "the loose sidecar is chosen, exactly as it was before .zip joined the scan"
    );

    for d in [library, data_base] {
        let _ = std::fs::remove_dir_all(&d);
    }
}
