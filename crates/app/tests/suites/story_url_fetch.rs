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
    let mut by_ext: std::collections::BTreeMap<String, PathBuf> = std::collections::BTreeMap::new();
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
        // Cheap: skip anything implausibly large for a story or a floppy.
        if entry.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > 16 * 1024 * 1024 {
            continue;
        }
        by_ext.entry(ext).or_insert(path);
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
    for (ext, path) in &by_ext {
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
    }
    assert!(opened > 0, "non-vacuity: at least one real specimen must have been exercised");
    eprintln!("round-tripped {opened} specimens of {} candidate formats", by_ext.len());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&probe);
}
