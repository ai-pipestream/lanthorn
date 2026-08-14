//! Where a story's assets come from — **one enumeration over every place a
//! story's files can live** (SQ-0843).
//!
//! # The defect this exists to end
//!
//! SQ-0840 unified how a disk image is *mounted and queried*: one table in
//! `blorb::medium`, one vocabulary, and no front-end that names a format. What
//! it could not unify is code that never mounted anything — and the art
//! enumeration in [`crate::launch_options`] was exactly that, a `read_dir` of
//! the story's own directory, written before babelmap could open a disk at all.
//!
//! So a story opened out of `stories/Zork Zero Disk.image` had **no** pickable
//! artwork: the two archives it wants are on the volume, and a directory scan
//! cannot see inside one. The Macintosh's two-colour `Pic.data` was reachable
//! only by typing its name into `--pictures` or the per-game `pictures` key.
//!
//! The user's rule, 2026-08-13: *"let's make sure we unify how this works
//! across all disk formats so we are not repeating ourselves"*.
//!
//! # The shape, and the property it buys
//!
//! [`files`] answers *"which files can this story's assets be read from?"*, and
//! it unions every SOURCE a story has: the host directory beside it, and the
//! volume it was mounted out of when it came off a disk image. Callers filter
//! that one list for the asset kind they want.
//!
//! Three kinds of growth, and each costs exactly one edit:
//!
//! | what is added | where it goes |
//! | --- | --- |
//! | a disk **format** | a row in `blorb::medium::FORMATS` (SQ-0840) |
//! | an asset **source** | an arm in [`files`], below |
//! | an asset **kind** | a filter over [`files`], written once |
//!
//! **No caller learns that disk images exist.** That is the whole point:
//! `launch_options` asks this module what files there are and applies its own
//! "is this a picture archive?" test to each, so the next asset kind that needs
//! the same answer inherits disk support rather than re-deriving it.
//!
//! # What this module does NOT do
//!
//! It does not migrate the hint-sidecar directory scan (`hints.rs`), and that is
//! deliberate: no disk in the corpus carries a hint sidecar, the Solid Gold
//! releases carry their hints *inside the story file* where `hints.rs` already
//! mounts to reach them, and building for a case with no example is the
//! speculative generality this project's rules forbid. The seam is shaped so
//! that job would be a filter and not a rewrite; it is not done here.
//!
//! It also does not identify anything. A file is a name, a place and some bytes;
//! whether those bytes are artwork is the caller's test, applied identically to
//! every source — which is what stops a volume's files being classified by one
//! rule and a directory's by another.
//!
//! # Cost
//!
//! The loose arm is **name-first**: it lists a directory and reads nothing, so a
//! caller can decline a file on its name before paying for its megabytes. That
//! property is load-bearing — the story browser's info panel asks per highlight,
//! and a flat library holds a dozen archives.
//!
//! The medium arm cannot be name-first, because a volume must be mounted before
//! it can be listed. It costs one read of the story file plus, when that file
//! really is a disk image, the mount. Measured on `stories/Zork Zero Disk.image`
//! (838 KB, DiskCopy 4.2 around HFS, five files): **0.2 ms** to read and sniff,
//! **0.5 ms** to mount and list — against 6.4 ms cold / 1.5 ms warm for the
//! whole of [`crate::launch_options::discover_art_candidates`] on that story,
//! most of which is parsing two Huffman picture directories. A story that is not
//! a disk image pays only the read and the sniff — ~0.3 ms for a 300 KB `.z6` —
//! because [`blorb::medium::DiskImage::detect`] refuses it before anything is
//! mounted.
//!
//! The story browser's info panel asks once per highlight, into the `StoryAux`
//! cache that already holds everything per-story that touches the disk, so none
//! of this is per frame.

use std::path::{Path, PathBuf};

/// Where one candidate file lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetOrigin {
    /// A loose file in the story's own directory.
    BesideTheStory,
    /// A file on the release disk image the story was mounted out of. Its
    /// identity is guaranteed by the medium — it shipped with the story — which
    /// is exactly what a loose file's name has to be tested for.
    OnTheMedium,
}

/// One file a story's assets could be read from, wherever it lives.
///
/// The bytes are deliberately *not* a public field. A loose file has not been
/// read yet and must not be until a caller decides it wants it; a file off a
/// volume was read by the mount and is already in hand. [`AssetFile::into_bytes`]
/// is the one door, and it hides which of those two happened.
#[derive(Debug, Clone)]
pub struct AssetFile {
    /// The filename, exactly as the directory or the volume spells it — which on
    /// FAT12, the one format with directories, includes the folder
    /// (`HITCHHIK/STORY.DAT`). This is what goes into a `pictures = "…"` key:
    /// both doors resolve the name against the story, the host filesystem first
    /// and then the medium (see `crate::graphics::read_off_the_medium`), so
    /// whatever is shown here can be asked for.
    pub name: String,
    /// The host path to open: the file itself when it is loose, and the disk
    /// image when it is on a volume — which is the file a person opened either
    /// way, and the only one that exists on this machine.
    pub path: PathBuf,
    /// Which source it came from.
    pub origin: AssetOrigin,
    /// Already in hand for a volume's file; `None` for a loose one, which is
    /// what keeps the directory arm name-first.
    bytes: Option<Vec<u8>>,
}

impl AssetFile {
    /// Is this file inside the medium rather than beside the story?
    pub fn is_on_medium(&self) -> bool {
        self.origin == AssetOrigin::OnTheMedium
    }

    /// The file's bytes, read now if they were not read already. `None` when a
    /// loose file will not read — a caller that cannot use it simply skips it.
    pub fn into_bytes(self) -> Option<Vec<u8>> {
        match self.bytes {
            Some(bytes) => Some(bytes),
            None => std::fs::read(&self.path).ok(),
        }
    }
}

/// Every file `story_path`'s assets could be read from: the loose files beside
/// it, then the files on the volume it was mounted out of.
///
/// Loose files come first and in directory order; the caller sorts. Nothing is
/// identified and nothing is filtered — see the module header.
pub fn files(story_path: &Path) -> Vec<AssetFile> {
    let mut out = beside_the_story(story_path);
    out.extend(on_the_medium(story_path));
    out
}

/// Source 1: the story's own directory, listed and not read.
fn beside_the_story(story_path: &Path) -> Vec<AssetFile> {
    let dir = match story_path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    rd.flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_name()?.to_str()?.to_string();
            Some(AssetFile { name, path, origin: AssetOrigin::BesideTheStory, bytes: None })
        })
        .collect()
}

/// Source 2: the release disk image, when the story came out of one.
///
/// Content, not extension, and one mount path for every format — the sniff and
/// the open are both `blorb::medium`'s, so a format babelmap can detect is a
/// format this arm enumerates, and adding one touches nothing here (SQ-0840).
///
/// A story that is not an image costs the read and the sniff and stops.
fn on_the_medium(story_path: &Path) -> Vec<AssetFile> {
    let Ok(raw) = std::fs::read(story_path) else {
        return Vec::new();
    };
    if blorb::medium::DiskImage::detect(&raw).is_none() {
        return Vec::new();
    }
    let Ok(disk) = blorb::medium::MountedDisk::mount(raw) else {
        return Vec::new();
    };
    disk.contents()
        .into_iter()
        .map(|(name, bytes)| AssetFile {
            name,
            path: story_path.to_path_buf(),
            origin: AssetOrigin::OnTheMedium,
            bytes: Some(bytes),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("babelmap-assets-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_loose_file_is_listed_without_being_read() {
        let dir = tmp("loose");
        std::fs::write(dir.join("story.z6"), b"x").unwrap();
        std::fs::write(dir.join("art.mg1"), b"pretend archive").unwrap();
        std::fs::create_dir_all(dir.join("subdir")).unwrap();
        let found = files(&dir.join("story.z6"));
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"art.mg1"), "{names:?}");
        assert!(names.contains(&"story.z6"), "the story itself is a file like any other: {names:?}");
        assert!(!names.contains(&"subdir"), "a directory is not an asset file: {names:?}");
        for f in &found {
            assert_eq!(f.origin, AssetOrigin::BesideTheStory);
            assert!(!f.is_on_medium());
            assert!(f.bytes.is_none(), "{}: the directory arm must stay name-first", f.name);
        }
        let art = found.into_iter().find(|f| f.name == "art.mg1").unwrap();
        assert_eq!(art.into_bytes().as_deref(), Some(&b"pretend archive"[..]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A story that is not release media mounts nothing, so nothing on the
    /// medium arm can cost it anything.
    #[test]
    fn an_ordinary_story_has_no_medium_arm_at_all() {
        let dir = tmp("nomedium");
        std::fs::write(dir.join("story.z6"), b"not a disk image").unwrap();
        assert!(files(&dir.join("story.z6")).iter().all(|f| !f.is_on_medium()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The seam over a real volume: every file on the disk arrives with its
    /// bytes already in hand, named as the volume spells it, pointing at the
    /// image a person actually opened.
    #[test]
    fn a_disk_images_own_files_are_listed_beside_the_loose_ones() {
        let image = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../stories/Zork Zero Disk.image");
        if !image.is_file() {
            eprintln!("SKIP: gitignored Macintosh medium missing at {}", image.display());
            return;
        }
        let found = files(&image);
        let on_disk: Vec<&AssetFile> = found.iter().filter(|f| f.is_on_medium()).collect();
        let names: Vec<&str> = on_disk.iter().map(|f| f.name.as_str()).collect();
        // The volume listing SQ-0838 measured, in full.
        for want in ["CPic.data", "Pic.data", "Story.data", "Zork Zero", "Desktop"] {
            assert!(names.contains(&want), "{want} is on the volume: {names:?}");
        }
        for f in &on_disk {
            assert_eq!(f.path, image, "a volume's file points at the image that carries it");
            assert!(f.bytes.is_some(), "{}: the mount already read it", f.name);
        }
        // …and the loose arm is still there, unchanged, in the same list.
        assert!(
            found.iter().any(|f| !f.is_on_medium() && f.name.eq_ignore_ascii_case("zork0.mg1")),
            "the story's own directory is still enumerated"
        );
    }
}
