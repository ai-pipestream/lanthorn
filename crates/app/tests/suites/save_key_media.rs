//! SQ-0850: the per-game save directory, measured on the real corpus.
//!
//! One image held one game for as long as `stories/` held only single-title
//! floppies, and keying a game's saves on the image's *filename* was safe by
//! accident. It is not any more: `Infocom Compilation 1 (19xx)(-).st` carries
//! six games, `floppy2.ima` carries six more, and every one of them resolved to
//! the same `<image>.save/` — one `default.lanthorn` for six stories, each
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

use app::storage::{DiskBuild, StoryOrigin, story_key_at, story_key_for};

// ── Fixtures ─────────────────────────────────────────────────────────────────

/// The key a story mounted off `path` with `build` takes. A disk image names
/// its story by the build, so there is no container entry to state here — see
/// [`StoryOrigin`], whose whole point is that the third fact has to be stated
/// rather than forgotten (SQ-1098).
fn key_off_disk(path: &Path, build: Option<&DiskBuild>) -> String {
    story_key_for(StoryOrigin { path, entry: None, build })
}

/// The key the bare container path takes with nothing else known — what every
/// story on a compilation used to share, and what none of them may take now.
fn key_of_container(path: &Path) -> String {
    story_key_for(StoryOrigin { path, entry: None, build: None })
}

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
    // An Apple IIgs ProDOS compilation (SQ-0836) — and specifically the volume
    // whose fifth game only became visible when `looks_like_story` learned to
    // read a high-ASCII serial (SQ-0856). `LEATHRGODDESSES` is the corpus's one
    // story whose serial is not six ASCII digits, so it is the only real medium
    // that can catch `DiskBuild::of` disagreeing with `blorb` about what a
    // serial is — a disagreement that lands it on the basename fallback, which
    // is what the case below forbids.
    "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg",
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
    let Some(kind) = blorb::medium::DiskImage::detect(&raw) else {
        return Vec::new();
    };
    let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { return Vec::new() };
    disk.stories().into_iter().map(|s| (s.name, DiskBuild::of(&s.bytes, kind))).collect()
}

// ── Guard 2: a mounted story always has a build ──────────────────────────────

/// A disk-mounted story must never fall back to the basename. The fallback
/// exists for bytes with no Z-machine header — a Glulx or Scott image — and
/// every format lanthorn mounts is an Infocom Z-code press, so on this corpus
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
            let key = key_off_disk(&path, Some(&build));
            assert_ne!(
                key,
                key_of_container(&path),
                "{image}: {name} must not key on the image's own filename",
            );
            // `disk_story_key` writes the serial with everything non-alphanumeric
            // turned into `_`, because it is a directory name. That was invisible
            // while every serial in the corpus was six digits; `LEATHRGODDESSES`
            // off `INFOCOM6` is `Blown!` and keys as `sBlown_` (SQ-0856).
            let serial: String = build
                .serial
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            let build_part = format!("-r{}-s{serial}", build.release);
            // **A Version 6 key carries its MEDIUM after the build** (SQ-1068).
            // One build can be pressed onto two disks — Arthur r54/890606 is on
            // the Amiga floppy and on the Macintosh Masterpieces volume — and for
            // v6 those are two machines, whose archives are not interchangeable:
            // the screen is stored in native pixels and the palette with it.
            // v1-v5 keys are unchanged and medium-agnostic, which is what guard 4
            // below still pins.
            if build.version == 6 {
                let want = format!("{build_part}-{}", build.medium.label().to_ascii_lowercase());
                assert!(key.ends_with(&want), "{image}: {name} -> {key} (wanted …{want})");
            } else {
                assert!(key.ends_with(&build_part), "{image}: {name} -> {key}");
            }
        }
    }
    assert!(ran > 0 || !any_media_present(), "no story mounted off any image in IMAGES");
}

// ── Guard 3: two games on one image, two directories ─────────────────────────

/// The defect itself. Every story on a compilation must land in its own
/// directory; before this change all six on an ST disk shared one, and whichever
/// game was played last owned `default.lanthorn`.
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
            let dir = app::storage::game_dir(base, &key_off_disk(&path, build.as_ref()));
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
                keys.push((image.to_string(), key_off_disk(&path, Some(&b))));
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
            let dir = app::storage::game_dir(base, &key_off_disk(&path, Some(&b)));
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

// ── The volume whose story is not on it ──────────────────────────────────────

/// A volume whose story comes from the RELEASE rather than from itself must key
/// the same through the path-only door as through the launch path (SQ-0952).
///
/// `story_key_at` reads the file and mounts it; `startup.rs` keys on the build it
/// actually loaded. Those agreed for every volume that carries its own story and
/// disagreed for every volume that does not, because the path-only door mounted
/// the PLATTER while the launch path mounts the SET. One game, two identities,
/// decided by which door the caller came in.
///
/// **`both_front_ends_name_one_directory` below cannot see this**, and that is
/// worth stating rather than leaving to be rediscovered: it mounts the platter,
/// asks `disk.story()`, and `continue`s when there is none — so the exact volumes
/// this is about are the ones it silently skips.
///
/// It shows on the story list. `picker::metadata_title` reads a story's fetched
/// metadata out of the directory this key names, while `startup.rs` hands the
/// in-game pane the build-keyed one — and `metadata_title`'s own doc promises
/// that "the list and the pane cannot name the same game differently".
#[test]
fn a_volume_that_carries_no_story_keys_on_the_one_its_release_holds() {
    let mut ran = 0;
    let mut discriminating = 0;
    let Ok(rd) = std::fs::read_dir(stories_dir()) else {
        assert!(!any_media_present(), "stories/ is unreadable but media are present");
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(raw) = std::fs::read(&path) else { continue };
        let Some(kind) = blorb::medium::DiskImage::detect(&raw) else { continue };
        // What the LAUNCH path would load: the set, not the platter.
        let Ok(set) = app::disk_set::mount_at(&path, raw.clone()) else { continue };
        let Some(story) = set.story() else { continue };
        ran += 1;

        // Is this one of the volumes the quest is about — a disk with no story
        // of its own? Counted so the case cannot pass vacuously on a shelf where
        // every image happens to carry its own.
        let platter_has_none = blorb::medium::MountedDisk::mount(raw)
            .ok()
            .and_then(|d| d.story())
            .is_none();
        if platter_has_none {
            discriminating += 1;
        }

        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        assert_eq!(
            story_key_at(&path),
            key_off_disk(&path, DiskBuild::of(&story.bytes, kind).as_ref()),
            "{name}: the path-only door and the launch path must name one directory\
             (this volume's story {} on the platter itself)",
            if platter_has_none { "is NOT" } else { "is" },
        );
    }
    assert!(ran > 0 || !any_media_present(), "no mountable disk image present");
    assert!(
        discriminating > 0 || !any_media_present(),
        "every image on this shelf carries its own story, so this case proved nothing — \
         it needs a volume whose story comes from a sibling (a DOS disk 1, or a paged .dsk)",
    );
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
        let Some(kind) = blorb::medium::DiskImage::detect(&raw) else { continue };
        let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else { continue };
        let Some(story) = disk.story() else { continue };
        ran += 1;
        let cli = cli_host::storage::game_dir_with_key(
            &path,
            Some("/data"),
            // `cli_host`'s own function, deliberately — the point of the case is
            // that the two crates apply one rule, so the TUI's helper above is
            // not what this side may call.
            &cli_host::storage::story_key_for(cli_host::storage::StoryOrigin {
                path: &path,
                entry: None,
                build: DiskBuild::of(&story.bytes, kind).as_ref(),
            }),
        );
        let tui = app::storage::game_dir(Path::new("/data"), &story_key_at(&path));
        assert_eq!(tui, cli, "{image}: the TUI and zvm-cli must agree");
    }
    assert!(ran > 0 || !any_media_present(), "no mountable image present in IMAGES");
}
