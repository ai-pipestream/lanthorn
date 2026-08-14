//! SQ-0850: the per-game save directory, measured on the real corpus.
//!
//! One image held one game for as long as `stories/` held only single-title
//! floppies, and keying a game's saves on the image's *filename* was safe by
//! accident. It is not any more: `Infocom Compilation 1 (19xx)(-).st` carries
//! six games, `floppy2.ima` carries six more, and every one of them resolved to
//! the same `<image>.save/` — one `default.babelmap` for six stories, each
//! overwriting the last.
//!
//! The rule now has two halves ([`cli_host::storage`]): a **loose** story file
//! keys on its basename exactly as before, and a story **mounted out of a disk
//! image** keys on its own release and serial. The unit tests beside the rule
//! pin its arithmetic; this file pins it against the media, which is the only
//! place the interesting cases actually exist — the same build pressed onto
//! three different disks, three different builds of one game, and thirty-odd
//! stories that must land in thirty-odd directories.
//!
//! `stories/` is gitignored (commercial media), so every case here skips
//! vacuously when its fixture is missing; the `ran > 0` guards that catch a
//! locally drifted filename are gated on [`any_media_present`] so they cannot
//! fire on CI, which legitimately has none of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use app::storage::{DiskBuild, story_key_at, story_key_for};

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Every disk image in `stories/` this suite reasons about, mountable or not —
/// the mount decides, never the extension. A name that is not here is simply not
/// covered; a name that is here and missing is a skip.
const IMAGES: &[&str] = &[
    // Atari ST compilations: the many-games-on-one-disk case (SQ-0835).
    "Infocom Compilation 1 (19xx)(-).st",
    "Infocom Compilation 6 (19xx)(-).st",
    "Infocom Compilation 9 (19xx)(-).st",
    // The Lost Treasures DOS pressing (SQ-0833).
    "floppy1.ima",
    "floppy2.ima",
    "floppy5.ima",
    "disk3.img",
    // Single-title release floppies, one game each.
    "Zork I - The Great Underground Empire.adf",
    "Zork Zero - The Revenge of Megaboz.adf",
    "Zork Zero Disk.image",
    "Journey - The Quest Begins.adf",
    "Arthur - The Quest for Excalibur.adf",
];

/// Loose story files whose save directory must not move, ever.
const LOOSE: &[&str] = &[
    "zork1-r88-s840726.z3",
    "zork0-r393-s890714.z6",
    "journey-r83-s890706.z6",
    "arthur-r74-s890714.z6",
    "beyondzork-r57-s871221.z5",
];

fn any_media_present() -> bool {
    IMAGES.iter().chain(LOOSE.iter()).any(|f| stories_dir().join(f).exists())
}

/// Every story on `image`, as `(stored name, build)` — the mount's own listing,
/// so a compilation yields all of its games and not just the one a bare path
/// would open. Empty when the file is absent or is not a disk image.
fn stories_on(image: &Path) -> Vec<(String, Option<DiskBuild>)> {
    let Ok(raw) = std::fs::read(image) else { return Vec::new() };
    if blorb::medium::DiskImage::detect(&raw).is_none() {
        return Vec::new();
    }
    let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { return Vec::new() };
    disk.stories().into_iter().map(|s| (s.name, DiskBuild::of(&s.bytes))).collect()
}

// ── Guard 2: a mounted story always has a build ──────────────────────────────

/// A disk-mounted story must never fall back to the basename. The fallback
/// exists for bytes with no Z-machine header — a Glulx or Scott image — and
/// every format babelmap mounts is an Infocom Z-code press, so on this corpus
/// nothing may reach it. Asserted rather than assumed: a mounted story that fell
/// back would silently rejoin its disk-mates in one directory, which is the
/// original defect wearing a different hat.
#[test]
fn every_story_on_every_image_keys_on_a_build() {
    let mut ran = 0;
    for image in IMAGES {
        let path = stories_dir().join(image);
        for (name, build) in stories_on(&path) {
            ran += 1;
            let build = build.unwrap_or_else(|| {
                panic!("{image}: {name} mounted with no readable Z header — it would key on the image's filename and collide with its disk-mates")
            });
            let key = story_key_for(&path, Some(&build));
            assert_ne!(
                key,
                story_key_for(&path, None),
                "{image}: {name} must not key on the image's own filename",
            );
            assert!(
                key.ends_with(&format!("-r{}-s{}", build.release, build.serial)),
                "{image}: {name} -> {key}",
            );
        }
    }
    assert!(ran > 0 || !any_media_present(), "no story mounted off any image in IMAGES");
}

// ── Guard 3: two games on one image, two directories ─────────────────────────

/// The defect itself. Every story on a compilation must land in its own
/// directory; before this change all six on an ST disk shared one, and whichever
/// game was played last owned `default.babelmap`.
#[test]
fn two_games_on_one_image_never_share_a_directory() {
    let base = Path::new("/data");
    let mut ran = 0;
    for image in IMAGES {
        let path = stories_dir().join(image);
        let stories = stories_on(&path);
        if stories.len() < 2 {
            continue; // a single-title floppy has nothing to collide with
        }
        ran += 1;
        let mut seen: BTreeMap<PathBuf, String> = BTreeMap::new();
        for (name, build) in &stories {
            let dir = app::storage::game_dir(base, &story_key_for(&path, build.as_ref()));
            if let Some(other) = seen.insert(dir.clone(), name.clone()) {
                panic!("{image}: {name} and {other} both resolve to {}", dir.display());
            }
        }
        assert_eq!(seen.len(), stories.len(), "{image}: {} games", stories.len());
    }
    assert!(ran > 0 || !any_media_present(), "no multi-story image present in IMAGES");
}

// ── Guard 4: one build, many images, one directory ───────────────────────────

/// *Zork I* release 88 / serial 840726 ships on the Amiga floppy, on the DOS
/// `floppy1.ima`, and on the Atari ST `Infocom Compilation 6` — the same build,
/// byte identity aside, three times over. It is one game with one set of saves,
/// and that is what makes the key survive renaming an image or a game moving
/// between disks in a set.
#[test]
fn one_build_across_three_media_resolves_to_one_directory() {
    let media = [
        "Zork I - The Great Underground Empire.adf",
        "floppy1.ima",
        "Infocom Compilation 6 (19xx)(-).st",
    ];
    let mut keys: Vec<(String, String)> = Vec::new();
    for image in media {
        let path = stories_dir().join(image);
        for (_, build) in stories_on(&path) {
            let Some(b) = build else { continue };
            if (b.release, b.serial.as_str()) == (88, "840726") {
                keys.push((image.to_string(), story_key_for(&path, Some(&b))));
            }
        }
    }
    if keys.len() < 2 {
        assert!(
            keys.len() == 1 || !any_media_present() || media.iter().all(|m| !stories_dir().join(m).exists()),
            "Zork I r88/840726 should be on each present medium, found {keys:?}",
        );
        return;
    }
    let first = &keys[0].1;
    for (image, key) in &keys {
        assert_eq!(key, first, "{image} names Zork I r88/840726 differently: {keys:?}");
    }
}

// ── Guard 5: different builds stay apart ─────────────────────────────────────

/// The project's standing rule — *a disk image is a different release, not the
/// same story on other media* — expressed as directories. Zork Zero's three
/// media are r296/881019 (Macintosh), r366/890323 (Amiga) and r393/890714 (DOS
/// `floppy5.ima`), and the bare `zork0-r393-s890714.z6` beside them is a LOOSE
/// file that keeps its basename. Four fixtures, four directories.
#[test]
fn the_zork_zero_builds_never_collide() {
    let base = Path::new("/data");
    let mut dirs: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut ran = 0;

    for image in ["Zork Zero Disk.image", "Zork Zero - The Revenge of Megaboz.adf", "floppy5.ima"] {
        let path = stories_dir().join(image);
        for (name, build) in stories_on(&path) {
            let Some(b) = build else { continue };
            if b.release < 200 {
                continue; // floppy5.ima carries only Zork Zero, but be explicit
            }
            ran += 1;
            let dir = app::storage::game_dir(base, &story_key_for(&path, Some(&b)));
            if let Some(other) = dirs.insert(dir.clone(), format!("{image}:{name}")) {
                panic!("{image}:{name} collides with {other} at {}", dir.display());
            }
        }
    }

    let loose = stories_dir().join("zork0-r393-s890714.z6");
    if loose.exists() {
        ran += 1;
        let dir = app::storage::game_dir(base, &story_key_at(&loose));
        assert_eq!(
            dir,
            base.join("zork0-r393-s890714.z6.save"),
            "a loose story file keeps its basename directory",
        );
        assert!(
            dirs.insert(dir.clone(), "loose zork0".into()).is_none(),
            "the loose file must not land on a floppy's directory: {}",
            dir.display(),
        );
    }
    assert!(ran > 0 || !any_media_present(), "no Zork Zero medium present");
}

// ── Guard 1: no existing save is orphaned ────────────────────────────────────

/// The promise. A loose `.z3`/`.z5`/`.z6`/`.zblorb` resolves to exactly the
/// directory it resolved to before SQ-0850 — its sanitized basename — so nobody
/// who already has saves loses sight of them. Measured through the path-only
/// door (`story_key_at`), which is the one that reads the file and could have
/// mistaken a story for a disk.
#[test]
fn a_loose_story_files_directory_does_not_move() {
    let mut ran = 0;
    for name in LOOSE {
        let path = stories_dir().join(name);
        if !path.exists() {
            continue;
        }
        ran += 1;
        assert_eq!(
            story_key_at(&path),
            *name,
            "{name}: a loose story file keys on its basename, unchanged",
        );
    }
    // Every `.z*` sitting loose in `stories/`, not just the named ones — the
    // promise is not limited to a list somebody remembered to write down.
    if let Ok(rd) = std::fs::read_dir(stories_dir()) {
        for e in rd.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            if !matches!(ext.as_str(), "z3" | "z4" | "z5" | "z6" | "z8" | "zblorb") {
                continue;
            }
            ran += 1;
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
            assert_eq!(story_key_at(&p), name, "{name} must keep its basename directory");
        }
    }
    assert!(ran > 0 || !any_media_present(), "no loose story file present");
}

// ── One key, both front-ends ─────────────────────────────────────────────────

/// `zvm-cli` and the TUI must reach the same directory for the same game off the
/// same disk — the user asked for the CLI to gain this behaviour, and two
/// implementations would have been two answers. There is one helper, and this is
/// the assertion that the TUI's path-only door lands where the CLI's
/// chosen-story door does for the story a bare path opens.
#[test]
fn both_front_ends_name_one_directory() {
    let mut ran = 0;
    for image in IMAGES {
        let path = stories_dir().join(image);
        let Ok(raw) = std::fs::read(&path) else { continue };
        if blorb::medium::DiskImage::detect(&raw).is_none() {
            continue;
        }
        let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { continue };
        let Some(story) = disk.story() else { continue };
        ran += 1;
        let cli = cli_host::storage::game_dir_with_key(
            &path,
            Some("/data"),
            &cli_host::storage::story_key_for(&path, DiskBuild::of(&story.bytes).as_ref()),
        );
        let tui = app::storage::game_dir(Path::new("/data"), &story_key_at(&path));
        assert_eq!(tui, cli, "{image}: the TUI and zvm-cli must agree");
    }
    assert!(ran > 0 || !any_media_present(), "no mountable image present in IMAGES");
}
