//! Typefaces on the user's OWN disk images under `~/.lanthorn/`, reported for
//! display only (SQ-1038).
//!
//! This exists to make SQ-1038's fix visible: before it, `blorb::mac_font` and
//! `blorb::amiga_font` refused every proportional system face (`Glyph::rows` was
//! one byte per row, so nothing wider than 8px could be represented), and a
//! player with a mounted System 6 or Workbench disk beside their stories could
//! not tell. This module answers "what can lanthorn read off my own media" —
//! the same question [`crate::native_font::detected`] answers for a STORY's own
//! medium, asked instead of the disks a person keeps around for their own
//! reasons (a Workbench boot floppy, a System Startup disk) rather than any
//! one game's release.
//!
//! # Display-only, like [`crate::native_font::DiskFace`]
//!
//! Nothing downstream consumes a [`SystemFace`]. Choosing WHEN a system face is
//! used by the renderer is SQ-1037, and is not this module's job — this only
//! reports what is readable, mirroring `native_font::detected`'s own "it ends at
//! a person's eyes" property. There is accordingly no `used` field: no story
//! wires a system face into rendering yet, so every row here is "present" and
//! none is "in use".
//!
//! # One lookup, reused rather than rewritten
//!
//! Faces are found through [`blorb::mac_font::faces_in_fork`] and
//! [`blorb::amiga_font::faces_in_volume`] — the same parsers
//! `native_font::detected` calls for a story's own medium — rather than a third
//! copy of the fitness question. SQ-1011 shipped inert twice because a fitness
//! rule existed in two places and correcting one left the other; there is no
//! fitness rule here at all, only "does it parse", which is one function.
//!
//! # Every entry with a resource fork, not only `APPL`
//!
//! A Macintosh system disk's fonts live in `System Folder/System`, whose file
//! type is `ZSYS`, not `APPL` — [`blorb::mac_font::from_volume`] already scans
//! every fork for exactly this reason (its own doc: "Searches the `APPL`
//! entries" is about where an INFOCOM RELEASE puts its font, not where a
//! system disk does). This module does the same: every catalog entry with a
//! resource fork is checked, matching `crates/app/examples/font_scout.rs`'s
//! fix for the same gap.

use std::path::{Path, PathBuf};

/// One typeface found on one of the user's own disks under `~/.lanthorn/`.
///
/// Mirrors [`crate::native_font::DiskFace`]'s fields (name/width/height/
/// proportional) minus `used` — see the module docs for why there is no
/// `used` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemFace {
    /// The disk it came off — the filename directly under `~/.lanthorn/`, so a
    /// face that reads identically off two disks (Workbench 1.2 and 1.3 ship
    /// IDENTICAL font drawers) still names which one answered, rather than
    /// collapsing into a single ambiguous row.
    pub disk: String,
    /// How the medium names it — `FONT 396` on a Macintosh, the filename on an
    /// Amiga volume.
    pub name: String,
    /// The cell it is drawn for.
    pub width: u8,
    pub height: u8,
    /// Whether its advance actually varies — see
    /// [`blorb::bitmap_font::BitmapFont::proportional`].
    pub proportional: bool,
    /// The machine this disk speaks for, from the volume's own filesystem — an
    /// HFS volume is a Macintosh, anything else `blorb` mounts is an Amiga.
    ///
    /// A story is only ever drawn with ITS OWN machine's faces, so a Macintosh
    /// System disk has nothing to say about an Amiga release and vice versa.
    /// Reporting both against either would be the same "present but never used"
    /// confusion SQ-1018 was, one layer out.
    pub machine: crate::interpreter::InterpreterProfile,
}

/// Every typeface on every mountable disk image directly inside `dir`.
///
/// Quiet on anything short of a parsed face: an absent `dir`, an empty one, one
/// with files that are not disk images, or a disk image with no font all answer
/// an empty `Vec` rather than an error — a player with no system disks under
/// `~/.lanthorn/` must see no change at all (SQ-1038).
///
/// `dir` is a parameter rather than always [`user_media_dir`] so a test can
/// point this at a temp directory carrying a synthetic fixture instead of the
/// user's own machine, which this module must never depend on for a passing
/// test (`unit_tests/macfont.hfs` is the one committed here).
pub fn scan(dir: &Path) -> Vec<SystemFace> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_image = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| blorb::medium::image_extensions().any(|known| known.eq_ignore_ascii_case(ext)));
        if !is_image {
            continue;
        }
        let Some(disk) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        let Ok(bytes) = std::fs::read(&path) else { continue };
        out.extend(faces_on(&path, &disk, bytes));
    }
    out
}

/// Every face `bytes` (one disk image's contents, at `path` and named `disk`)
/// carries.
fn faces_on(path: &Path, disk: &str, bytes: Vec<u8>) -> Vec<SystemFace> {
    // A Macintosh volume: every resource fork on it, not only an `APPL`'s — see
    // the module docs for why (`ZSYS`, the System file, is where a system disk's
    // fonts live).
    if blorb::hfs::Hfs::looks_like_hfs(&bytes) {
        let Ok(hfs) = blorb::hfs::Hfs::mount(bytes) else { return Vec::new() };
        return hfs
            .files()
            .iter()
            .filter(|e| e.resource_size > 0)
            .filter_map(|e| hfs.read_resource(e))
            .filter_map(|fork| blorb::resource_fork::ResourceFork::parse(&fork))
            .flat_map(|rf| blorb::mac_font::faces_in_fork(&rf))
            .map(|(id, f)| SystemFace {
                disk: disk.to_string(),
                name: format!("FONT {id}"),
                width: f.width,
                height: f.height,
                proportional: f.proportional,
                machine: crate::interpreter::InterpreterProfile::Macintosh,
            })
            .collect();
    }
    // Every other medium blorb can mount: an AmigaDOS disk font is a file, named
    // by one, exactly as `native_font::detected` reads it.
    //
    // Through `cli_host::disk_set::mount_at`, not `blorb::medium::MountedDisk::
    // mount` directly — every front-end mounts a disk through that one seam (see
    // its own docs, and `release_enumeration::no_production_code_mounts_the_
    // platter_alone`), even though a font drawer never spans volumes the way a
    // paged Apple II release does: there is one seam, not one seam plus an
    // exception for callers that believe they don't need it.
    let Ok(mounted) = crate::disk_set::mount_at(path, bytes) else { return Vec::new() };
    let files = mounted.contents();
    blorb::amiga_font::faces_in_volume(files.iter().map(|(n, b)| (n.as_str(), b.as_slice())))
        .into_iter()
        .map(|(name, f)| SystemFace {
            disk: disk.to_string(),
            name,
            width: f.width,
            height: f.height,
            proportional: f.proportional,
            machine: crate::interpreter::InterpreterProfile::Amiga,
        })
        .collect()
}

/// `~/.lanthorn/` — the fixed spot a player drops their own system disks,
/// independent of `--user-dir` or `--data-dir`: those move where LANTHORN's OWN
/// state lives, not where a person's media sits. Same fallback as
/// `config::default_user_dir` (`$HOME`, or `.` when unset), kept as its own tiny
/// copy rather than threading `Config` through the picker's aux resolution just
/// for this.
pub fn user_media_dir() -> PathBuf {
    let base = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".lanthorn")
}

/// Every typeface on the user's own disks, off [`user_media_dir`].
pub fn detected() -> Vec<SystemFace> {
    scan(&user_media_dir())
}

/// The faces on the user's own disks that `machine` could actually draw with.
///
/// # Why this filters rather than reporting everything
///
/// A face is only ever a candidate for the machine that owns it: Geneva off a
/// Macintosh System disk says nothing about an Amiga release, and topaz off a
/// Workbench floppy says nothing about a Macintosh one. Listing both against
/// either would put rows in front of a reader that can never apply to the story
/// they are looking at — which is the "present but never used" confusion SQ-1018
/// cost a bug report for, and this panel exists partly to prevent.
///
/// The caller is also responsible for asking only about a **Version 6** story.
/// Nothing below v6 draws text from a disk face at all — v1-v5 text goes through
/// the terminal, so a system disk is irrelevant there whatever machine it names.
pub fn detected_for(machine: crate::interpreter::InterpreterProfile) -> Vec<SystemFace> {
    scan_for(&user_media_dir(), machine)
}

/// [`scan`], keeping only the faces `machine` could draw with. Split out from
/// [`detected_for`] so the filter is testable against a directory of our own
/// rather than whatever the person running the tests keeps in `~/.lanthorn/`.
pub fn scan_for(dir: &Path, machine: crate::interpreter::InterpreterProfile) -> Vec<SystemFace> {
    let mut out = scan(dir);
    out.retain(|f| f.machine == machine);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn macfont_hfs_bytes() -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../unit_tests/macfont.hfs");
        std::fs::read(&path).expect("unit_tests/macfont.hfs is committed and readable")
    }

    /// A Macintosh disk answers a Macintosh story and says nothing to an Amiga
    /// one (SQ-1038).
    ///
    /// The filter matters because a face that can never be drawn is worse than no
    /// row at all: SQ-1018 was reported as crowded text and was really a face
    /// sitting present-and-unused, and a Geneva listed under a Journey floppy
    /// would be the same confusion one layer out.
    #[test]
    fn a_disk_only_answers_its_own_machine() {
        let dir = std::env::temp_dir().join(format!("sq1038-machine-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("System.img"), macfont_hfs_bytes()).expect("write");

        let mac = scan_for(&dir, crate::interpreter::InterpreterProfile::Macintosh);
        assert!(!mac.is_empty(), "the Macintosh disk answers a Macintosh story");
        assert!(
            mac.iter().all(|f| f.machine == crate::interpreter::InterpreterProfile::Macintosh),
            "and every row it answers with is its own machine's: {mac:?}",
        );

        // Non-vacuity: the unfiltered scan really does find these, so the empty
        // answer below is the FILTER working and not an unreadable fixture.
        assert_eq!(scan(&dir).len(), mac.len(), "the filter kept everything the scan found");

        for other in [
            crate::interpreter::InterpreterProfile::Amiga,
            crate::interpreter::InterpreterProfile::IbmPc,
        ] {
            assert_eq!(scan_for(&dir, other), Vec::new(), "{other:?} sees nothing on a Mac disk");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An absent directory answers an empty list quietly — no error, no panic —
    /// which is what a player with no `~/.lanthorn/` at all must see.
    #[test]
    fn an_absent_directory_is_quiet() {
        let dir = std::env::temp_dir().join(format!("sq1038-absent-{}", std::process::id()));
        assert!(!dir.exists());
        assert_eq!(scan(&dir), Vec::new());
    }

    /// An existing but empty directory, and one with unrelated files, both
    /// answer empty too — nothing here treats "no fonts found" as an error.
    #[test]
    fn an_empty_or_irrelevant_directory_is_quiet() {
        let dir = std::env::temp_dir().join(format!("sq1038-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("notes.txt"), b"not a disk image").expect("write");
        std::fs::write(dir.join("config.toml"), b"honor_game_colours = true").expect("write");
        assert_eq!(scan(&dir), Vec::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The synthetic fixture parses to its two known faces, named with the disk
    /// they came off — the case this module exists to pass without depending on
    /// the user's own `~/.lanthorn/` (SQ-1038).
    #[test]
    fn a_synthetic_macintosh_disk_reports_its_faces_named_with_the_disk() {
        let dir = std::env::temp_dir().join(format!("sq1038-macfont-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("MyStartup.img"), macfont_hfs_bytes()).expect("write fixture");
        // Not an image extension — must be skipped even though the bytes inside
        // are perfectly good HFS, since the pre-filter is on the name.
        std::fs::write(dir.join("MyStartup.txt"), macfont_hfs_bytes()).expect("write decoy");

        let faces = scan(&dir);
        assert_eq!(faces.len(), 2, "FONT 524 and FONT 1033, and nothing from the .txt decoy: {faces:?}");
        assert!(faces.iter().all(|f| f.disk == "MyStartup.img"), "named with the disk: {faces:?}");
        let body = faces.iter().find(|f| f.name == "FONT 524").expect("FONT 524 is listed");
        assert_eq!((body.width, body.height), (7, 15));
        let alt = faces.iter().find(|f| f.name == "FONT 1033").expect("FONT 1033 is listed");
        assert_eq!((alt.width, alt.height), (7, 12));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two disks with the same face still report as two rows: the exact case
    /// Workbench 1.2 and 1.3 exercise on the user's own machine (identical font
    /// drawers), reproduced here with the one fixture this module may depend on.
    #[test]
    fn duplicate_faces_across_two_disks_both_report() {
        let dir = std::env::temp_dir().join(format!("sq1038-dup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("Disk1.img"), macfont_hfs_bytes()).expect("write");
        std::fs::write(dir.join("Disk2.img"), macfont_hfs_bytes()).expect("write");

        let faces = scan(&dir);
        assert_eq!(faces.len(), 4, "2 faces x 2 disks: {faces:?}");
        let disks: std::collections::BTreeSet<&str> = faces.iter().map(|f| f.disk.as_str()).collect();
        assert_eq!(disks, std::collections::BTreeSet::from(["Disk1.img", "Disk2.img"]));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [`user_media_dir`] resolves under `$HOME`, matching
    /// `config::default_user_dir`'s own fallback — pinned so the two cannot
    /// silently drift onto different homes.
    #[test]
    fn user_media_dir_is_under_home() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(user_media_dir(), PathBuf::from(home).join(".lanthorn"));
        }
    }
}
