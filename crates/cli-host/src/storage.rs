//! Where a story's saves and sidecars live.
//!
//! Both save-capable hosts answer the same three questions — what do I call this
//! game's directory, where is it, and what does a filename the player typed at a
//! save prompt resolve to — and both answered them with the same code, comments
//! included.
//!
//! That mattered more than the duplication usually does, because one of the
//! answers is a bug fix. The per-game directory used to be `base/<story name>`,
//! which collides with the story file itself whenever `base` is the story's own
//! directory: `mkdir` on an existing filename fails, so saving was impossible in
//! the most ordinary layout there is. The `.save` suffix (SQ-0284/0294) is what
//! separates them, and it was living in two places at once (SQ-0618).
//!
//! `scott-cli` has no part in this: Scott has no save protocol at all.
//!
//! ## Two rules, because one disk is no longer one game (SQ-0850)
//!
//! A **loose story file** is keyed by its basename, exactly as it always was.
//! That rule is a promise, not an implementation detail: every save anybody
//! already has sits in a directory named that way, and moving them would orphan
//! them.
//!
//! A story **mounted out of a disk image** is keyed by its own release and
//! serial instead — [`disk_story_key`]. The basename cannot answer for it: one
//! `Infocom Compilation 1 (19xx)(-).st` carries six games, and they would all
//! have shared one `default.babelmap` and overwritten each other. Keying on the
//! build gives three properties the filename never had — renaming the image
//! keeps the saves, a game that moves between disks in a set keeps them, and
//! two games on one disk cannot collide — and it is the same identity this
//! project already uses to say that a disk image is a *different release*
//! rather than the same story on other media.

use std::path::{Path, PathBuf};

/// The build a story mounted off a disk image is: the release and serial its
/// Z-machine header carries. This, not a filename, is what names its saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskBuild {
    /// Header `$02`, big-endian.
    pub release: u16,
    /// Header `$12..$18`, six ASCII bytes (`840726`), with bit 7 masked off —
    /// see [`DiskBuild::of`].
    pub serial: String,
}

impl DiskBuild {
    /// Read a story image's build, or `None` when `bytes` are not Z-code with a
    /// plausible header (a Glulx or Scott image, or something too short).
    ///
    /// The version-byte and serial checks are the ones `zvm-cli`'s disk menu
    /// already applies to the same bytes, so a candidate the menu can label is
    /// exactly a candidate this can key.
    ///
    /// **Bit 7 comes off each serial byte before it is read**, for the same
    /// reason `blorb::adf::looks_like_story` masks it (SQ-0856): the Apple II
    /// wrote text with the high bit set, and `LEATHRGODDESSES` on *Lost
    /// Treasures* volume `INFOCOM6` spells its serial `C2 EC EF F7 EE A1` —
    /// "Blown!". `blorb` offers that story, so this must be able to key it;
    /// a `None` here would send it to the basename fallback and back in with
    /// its disk-mates, which is the defect this whole module exists to fix.
    /// Every other serial in the corpus has bit 7 clear, so nothing else moves.
    pub fn of(bytes: &[u8]) -> Option<DiskBuild> {
        if bytes.len() < 0x18 || !(3..=8).contains(&bytes[0]) {
            return None;
        }
        let serial: String = bytes[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect();
        if !serial.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return None;
        }
        Some(DiskBuild { release: u16::from_be_bytes([bytes[0x02], bytes[0x03]]), serial })
    }
}

/// A story file's basename, sanitized into a directory-name token.
///
/// The **extension is kept**, deliberately: `Zork1.z5` and `Zork1.gblorb` are
/// different games as far as saves are concerned, and dropping it would let them
/// share a directory. Anything outside `[A-Za-z0-9._-]` becomes `_`, so a title
/// with spaces, quotes or a slash cannot escape the directory it names. An empty
/// result falls back to `game`.
pub fn story_key(story_path: &Path) -> String {
    let name = story_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let s: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect();
    if s.is_empty() { "game".to_string() } else { s }
}

/// Words that read as noise on the end of a truncated slug, and the articles a
/// title opens with. Dropped so `The Hitchhiker's Guide to the Galaxy` becomes
/// `hitchhikers-guide` rather than `hitchhikers-guide-to-the`.
const FILLER: &[&str] =
    &["a", "an", "the", "of", "to", "and", "or", "in", "for", "on", "at", "from"];

/// How long a slug may grow before it stops taking whole words.
const SLUG_MAX: usize = 24;

/// The readable half of a disk-mounted story's key.
///
/// The **identity** is the release and serial that follow it; this part exists so
/// a person reading `~/.babelmap` can tell `hitchhikers-guide-r59-s851108` from
/// `zork-i-r88-s840726` without opening either. It is derived mechanically from
/// the canonical title `cli_host::titles` already holds — a curated slug column
/// would be a second table to keep in step with the first — by dropping any
/// subtitle after the colon, lowercasing, turning runs of punctuation into a
/// single `-`, and taking whole words up to [`SLUG_MAX`] characters. `story` is
/// the answer for a build the table does not name, which is still unique because
/// the release and serial are doing the work.
fn slug_for(release: u16, serial: &str) -> String {
    let Some(title) = crate::titles::title_for_build(release, serial) else {
        return "story".to_string();
    };
    // The subtitle is where every Infocom title puts the words nobody says out
    // loud: `Arthur: The Quest for Excalibur` is `arthur`.
    let head = title.split(':').next().unwrap_or(title);
    // Apostrophes close up (`Hitchhiker's` → `hitchhikers`); everything else that
    // is not a letter or a digit is a word break.
    let flat: String = head
        .chars()
        .filter(|c| *c != '\'' && *c != '\u{2019}')
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    let mut words: Vec<&str> = flat.split_whitespace().collect();
    // Leading article, then whole words while they fit.
    if words.len() > 1 && FILLER.contains(&words[0]) {
        words.remove(0);
    }
    let mut out: Vec<&str> = Vec::new();
    let mut len = 0;
    for w in words {
        let next = if out.is_empty() { w.len() } else { len + 1 + w.len() };
        if next > SLUG_MAX && !out.is_empty() {
            break;
        }
        len = next;
        out.push(w);
    }
    // A truncation that stopped mid-phrase leaves connectives dangling.
    while out.len() > 1 && FILLER.contains(out.last().unwrap()) {
        out.pop();
    }
    let s = out.join("-");
    if s.is_empty() { "story".to_string() } else { s }
}

/// The per-game directory token for a story mounted out of a disk image:
/// `<slug>-r<release>-s<serial>`, e.g. `hitchhikers-guide-r59-s851108`.
///
/// Deterministic and machine-independent — every input is either the story's own
/// header or a table compiled into the binary — so the same build names the same
/// directory on every run and on every machine.
pub fn disk_story_key(build: &DiskBuild) -> String {
    let serial: String = build
        .serial
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}-r{}-s{serial}", slug_for(build.release, &build.serial), build.release)
}

/// The per-game directory token, by whichever of the two rules applies (see the
/// module docs): the story's build when it came off a disk image, its basename
/// when it is a loose file.
///
/// `build` is `None` for every loose story file, and for a disk-mounted story
/// whose bytes carry no Z-machine header — which cannot happen on the corpus, as
/// every mountable format here is Infocom Z-code, and is a basename fallback
/// rather than a failure if it ever does.
pub fn story_key_for(story_path: &Path, build: Option<&DiskBuild>) -> String {
    match build {
        Some(b) => disk_story_key(b),
        None => story_key(story_path),
    }
}

/// [`story_key_for`] for a caller holding nothing but the path: reads the file
/// and, when it really is a disk image, mounts it and keys on the story it would
/// open — the same tiebreak `app`'s loader and `zvm-cli`'s single-game path take,
/// so all three name one directory.
///
/// Costs a read of the file, so a caller that has already scanned the story
/// (the picker has its release and serial in hand) should call [`story_key_for`]
/// instead. An unreadable file is simply not a disk image.
pub fn story_key_at(story_path: &Path) -> String {
    story_key_at_from(story_path, None)
}

/// [`story_key_at`] for one **named** story off a disk image that holds several
/// (SQ-0859) — the game a front-end actually chose, rather than the tiebreak a
/// bare path resolves to.
///
/// `None` is exactly [`story_key_at`], which is what every loose story file and
/// every single-game floppy passes.
pub fn story_key_at_from(story_path: &Path, disk_entry: Option<&str>) -> String {
    story_key_for(story_path, mounted_build(story_path, disk_entry).as_ref())
}

/// The build `story_path` would open if it is a disk image, else `None`.
/// `disk_entry` names which story when the image holds several; without one the
/// format's own tiebreak decides, as it always did.
fn mounted_build(story_path: &Path, disk_entry: Option<&str>) -> Option<DiskBuild> {
    let raw = std::fs::read(story_path).ok()?;
    // `detect` first: `mount` consumes the bytes, and the overwhelming majority
    // of calls are about an ordinary story file.
    blorb::medium::DiskImage::detect(&raw)?;
    let disk = blorb::medium::MountedDisk::mount(raw).ok()?;
    let chosen = match disk_entry {
        Some(want) => disk
            .stories()
            .into_iter()
            .find(|s| s.name == want || s.name.eq_ignore_ascii_case(want))?,
        None => disk.story()?,
    };
    DiskBuild::of(&chosen.bytes)
}

/// The directory holding this story's saves and sidecars.
///
/// `data_dir` is the `--data-dir` override; without it the story's own directory
/// is used, which is what makes the `.save` suffix load-bearing rather than
/// decorative — see the module docs. A story path with no parent (a bare
/// filename) resolves against the current directory.
pub fn game_dir(story_path: &Path, data_dir: Option<&str>) -> PathBuf {
    game_dir_with_key(story_path, data_dir, &story_key(story_path))
}

/// [`game_dir`] for a caller that has already worked out the key — the mounted
/// case, where the story `--story` chose is not the one a bare path would open.
pub fn game_dir_with_key(story_path: &Path, data_dir: Option<&str>, key: &str) -> PathBuf {
    let base = data_dir.map(PathBuf::from).unwrap_or_else(|| {
        story_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    });
    base.join(format!("{key}.save"))
}

/// Resolve what the player typed at a save/restore filename prompt.
///
/// A bare name lands in `game_dir` with a `.qzl` extension, so `quick` means
/// this game's quick save and not a file in whatever directory the shell
/// happened to be in. Anything carrying a path separator is honoured verbatim,
/// which is the escape hatch for saving somewhere else on purpose.
pub fn resolve_save_input(input: &str, game_dir: &Path) -> PathBuf {
    let t = input.trim();
    if t.contains('/') || t.contains('\\') {
        PathBuf::from(t)
    } else {
        let name = if t.ends_with(".qzl") { t.to_string() } else { format!("{t}.qzl") };
        game_dir.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_key_keeps_the_extension_and_sanitizes_the_rest() {
        assert_eq!(story_key(Path::new("/g/Zork1.z5")), "Zork1.z5");
        assert_ne!(
            story_key(Path::new("/g/Zork1.z5")),
            story_key(Path::new("/g/Zork1.gblorb")),
            "two formats of the same title must not share a save directory"
        );
        assert_eq!(story_key(Path::new("/g/a b?.z5")), "a_b_.z5");
        assert_eq!(story_key(Path::new("")), "game");
    }

    #[test]
    fn a_hostile_filename_cannot_escape_the_directory_it_names() {
        // Only the basename is considered, and separators are not in the allowed
        // set, so nothing here can climb out.
        assert_eq!(story_key(Path::new("../../etc/passwd")), "passwd");
        assert!(!story_key(Path::new("/g/a/b.z5")).contains('/'));
        assert!(!story_key(Path::new(r"C:\g\b.z5")).contains('\\'));
    }

    // ── The disk-image key (SQ-0850) ─────────────────────────────────────────

    /// A Z-code header with the given release and serial, long enough for
    /// [`DiskBuild::of`] to read.
    fn header(version: u8, release: u16, serial: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x40];
        b[0] = version;
        b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
        b[0x12..0x18].copy_from_slice(serial.as_bytes());
        b
    }

    fn build(release: u16, serial: &str) -> DiskBuild {
        DiskBuild { release, serial: serial.to_string() }
    }

    /// **Guard 1, the promise.** A loose story file keys on its basename and
    /// nothing else, so no save anybody already has is orphaned by this change.
    #[test]
    fn a_loose_story_file_still_keys_on_its_basename() {
        for name in ["zork1-r88-s840726.z3", "Zork1.z5", "advent.gblorb", "a b?.z5"] {
            let p = PathBuf::from("/games").join(name);
            assert_eq!(
                story_key_for(&p, None),
                story_key(&p),
                "{name}: a loose file's key is its basename, unchanged"
            );
        }
        // And the whole directory, not merely the token.
        assert_eq!(
            game_dir(Path::new("/games/zork1-r88-s840726.z3"), None),
            PathBuf::from("/games/zork1-r88-s840726.z3.save"),
        );
    }

    /// The shape the key takes, and where each half comes from: a readable slug
    /// off the canonical-title table, then the identity that actually
    /// distinguishes builds.
    #[test]
    fn a_disk_story_keys_on_its_release_and_serial() {
        assert_eq!(disk_story_key(&build(59, "851108")), "hitchhikers-guide-r59-s851108");
        assert_eq!(disk_story_key(&build(88, "840726")), "zork-i-r88-s840726");
        assert_eq!(disk_story_key(&build(393, "890714")), "zork-zero-r393-s890714");
        // The subtitle every Infocom title carries is where the words nobody
        // says out loud live.
        assert_eq!(disk_story_key(&build(54, "890606")), "arthur-r54-s890606");
        assert_eq!(disk_story_key(&build(30, "890322")), "journey-r30-s890322");
        // A build the table does not name is still keyed by its identity.
        assert_eq!(disk_story_key(&build(1, "000000")), "story-r1-s000000");
    }

    /// **Guard 3.** Two games off one image cannot share a directory — the point
    /// of the change. These are the first two stories on
    /// `Infocom Compilation 1 (19xx)(-).st`, keyed from their headers alone.
    #[test]
    fn two_games_on_one_image_get_two_directories() {
        let image = Path::new("/games/Infocom Compilation 1 (19xx)(-).st");
        let hitchhikers = story_key_for(image, Some(&build(56, "841221")));
        let planetfall = story_key_for(image, Some(&build(29, "840118")));
        assert_ne!(hitchhikers, planetfall, "one image, one directory, was the defect");
        // …and neither is the image's own name, which is what they used to share.
        assert_ne!(hitchhikers, story_key(image));
        assert_ne!(planetfall, story_key(image));
    }

    /// **Guard 4.** The same build off two different images is one game with one
    /// set of saves: *Zork I* r88/840726 ships on the Amiga floppy, on DOS
    /// `floppy1.ima`, and on the Atari ST compilation. Renaming an image, or a
    /// game moving between disks in a set, must not lose them either.
    #[test]
    fn one_build_on_many_images_gets_one_directory() {
        let zork1 = build(88, "840726");
        let keys: Vec<String> = [
            "/games/Zork I - The Great Underground Empire.adf",
            "/games/floppy1.ima",
            "/games/Infocom Compilation 6 (19xx)(-).st",
            "/elsewhere/renamed.img",
        ]
        .iter()
        .map(|p| story_key_for(Path::new(p), Some(&zork1)))
        .collect();
        assert!(keys.windows(2).all(|w| w[0] == w[1]), "{keys:?}");
    }

    /// **Guard 5.** Different builds of the same game stay apart, which is the
    /// project's standing rule that a disk image is a different *release*. The
    /// three Zork Zero media are r296/881019 (Macintosh), r366/890323 (Amiga)
    /// and r393/890714 (DOS).
    #[test]
    fn different_builds_of_one_game_do_not_collide() {
        let keys = [build(296, "881019"), build(366, "890323"), build(393, "890714")]
            .map(|b| disk_story_key(&b));
        assert_eq!(keys.len(), 3);
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], keys[2]);
        // The bare `zork0-r393-s890714.z6` is a LOOSE file and keeps its
        // basename, so it cannot land on the DOS floppy's directory either.
        let loose = Path::new("/games/zork0-r393-s890714.z6");
        assert_ne!(story_key_for(loose, None), keys[2]);
    }

    /// A key is a directory name: no separator, no space, nothing a shell or a
    /// filesystem has an opinion about.
    #[test]
    fn a_disk_key_is_a_safe_directory_name() {
        for b in [build(59, "851108"), build(1, "a b/c\\d"), build(0, "")] {
            let k = disk_story_key(&b);
            assert!(
                k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{k:?}"
            );
        }
    }

    /// **Guard 2's fallback half.** Bytes with no Z-machine header — a Glulx or
    /// Scott image — have no build, so the key falls back to the basename. (That
    /// a *disk-mounted* story never reaches this is pinned on the real corpus in
    /// `app`'s `save_key_media` suite; every mountable format here is Infocom
    /// Z-code.)
    #[test]
    fn bytes_that_are_not_z_code_have_no_build() {
        assert_eq!(DiskBuild::of(b"Glul\0\0\0\0"), None, "Glulx magic, not a Z header");
        assert_eq!(DiskBuild::of(&[]), None);
        assert_eq!(DiskBuild::of(&header(2, 88, "840726")), None, "v2 is out of range");
        assert_eq!(DiskBuild::of(&header(9, 88, "840726")), None, "v9 is out of range");
        assert_eq!(
            DiskBuild::of(&header(3, 88, "840726")),
            Some(build(88, "840726")),
            "and a real v3 header reads",
        );
    }

    /// **A high-ASCII serial still keys** (SQ-0856). `LEATHRGODDESSES` off
    /// *Lost Treasures* `INFOCOM6` writes `C2 EC EF F7 EE A1` at `$12`, which is
    /// "Blown!" with bit 7 off. `blorb` offers that story now, so a `None` here
    /// would drop it into the basename fallback and back in with its disk-mates
    /// — guard 2's failure mode exactly.
    #[test]
    fn a_high_ascii_serial_still_keys_on_its_build() {
        let mut bytes = header(3, 0, "......");
        bytes[0x12..0x18].copy_from_slice(&[0xc2, 0xec, 0xef, 0xf7, 0xee, 0xa1]);
        assert_eq!(DiskBuild::of(&bytes), Some(build(0, "Blown!")));
        // No title in the table answers to release 0, so it slugs as `story`,
        // and `!` is not a directory character.
        assert_eq!(disk_story_key(&build(0, "Blown!")), "story-r0-sBlown_");
        // Binary is still binary with the bit masked: a saved game does not key.
        let mut save = header(3, 0, "......");
        save[0x12..0x18].copy_from_slice(&[0x00; 6]);
        assert_eq!(DiskBuild::of(&save), None, "an all-zero serial is not text");
    }

    /// `--story` can pick any game off a compilation, so `zvm-cli` works out the
    /// key itself and hands it over; the base still comes from the image's path
    /// or `--data-dir`, exactly as before.
    #[test]
    fn game_dir_with_key_places_a_chosen_story() {
        let image = Path::new("/games/Infocom Compilation 1 (19xx)(-).st");
        let key = disk_story_key(&build(29, "840118"));
        assert_eq!(
            game_dir_with_key(image, None, &key),
            PathBuf::from("/games/planetfall-r29-s840118.save"),
        );
        assert_eq!(
            game_dir_with_key(image, Some("/data"), &key),
            PathBuf::from("/data/planetfall-r29-s840118.save"),
        );
    }

    #[test]
    fn the_game_dir_does_not_collide_with_the_story_file_itself() {
        // SQ-0284/0294. The default base IS the story's own directory, so
        // without the `.save` suffix `mkdir` would be asked to create a
        // directory where the story file already is, and fail.
        let dir = game_dir(Path::new("/games/zork1.z5"), None);
        assert_eq!(dir, PathBuf::from("/games/zork1.z5.save"));
        assert_ne!(dir, PathBuf::from("/games/zork1.z5"), "must not be the story file");
    }

    #[test]
    fn the_game_dir_can_actually_be_created_beside_the_story_file() {
        // The path comparison above is not the whole guarantee: what failed
        // before SQ-0294 was `mkdir` itself, refusing to create a directory
        // where a file of that name already exists. So do it for real.
        let tmp = std::env::temp_dir()
            .join(format!("cli-host-storage-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let story_path = tmp.join("game.z5");
        std::fs::write(&story_path, b"x").unwrap(); // a FILE named game.z5

        let dir = game_dir(&story_path, None);
        assert_eq!(dir, tmp.join("game.z5.save"));
        std::fs::create_dir_all(&dir).expect("must not collide with the story file");
        assert!(dir.is_dir());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn data_dir_overrides_the_storys_own_directory() {
        let dir = game_dir(Path::new("/games/zork1.z5"), Some("/var/saves"));
        assert_eq!(dir, PathBuf::from("/var/saves/zork1.z5.save"));
    }

    #[test]
    fn a_bare_story_filename_resolves_against_the_current_directory() {
        assert_eq!(game_dir(Path::new("zork1.z5"), None), PathBuf::from("./zork1.z5.save"));
    }

    #[test]
    fn a_bare_save_name_lands_in_the_game_dir_a_path_does_not() {
        let gd = Path::new("/data/Zork1.z5.save");
        assert_eq!(resolve_save_input("quick", gd), PathBuf::from("/data/Zork1.z5.save/quick.qzl"));
        assert_eq!(
            resolve_save_input("quick.qzl", gd),
            PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "an extension the player typed is not doubled"
        );
        assert_eq!(resolve_save_input("/tmp/foo.qzl", gd), PathBuf::from("/tmp/foo.qzl"));
        // A RELATIVE path is the case that actually pins the rule: `Path::join`
        // with an absolute path replaces the whole thing, so the absolute case
        // above passes either way and cannot tell a working escape hatch from a
        // broken one.
        assert_eq!(
            resolve_save_input("saves/foo.qzl", gd),
            PathBuf::from("saves/foo.qzl"),
            "a path the player typed is honoured, not reparented into the game dir"
        );
        assert_eq!(resolve_save_input("  quick  ", gd), PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "trimmed, because the prompt line carries whatever was typed");
    }
}
