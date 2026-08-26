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
//! have shared one `default.lanthorn` and overwritten each other. Keying on the
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
    /// Header `$00` — the Z-machine version.
    ///
    /// Carried because it decides whether the MEDIUM belongs in the key: a v1-v5
    /// save is VM memory and says nothing about the machine, where a Version 6
    /// archive carries the screen in NATIVE PIXELS plus its palette. See
    /// [`disk_story_key`].
    pub version: u8,
    /// Which kind of disk this build came off — the same fact the story list
    /// shows in its TYPE parenthetical (`Z6 (ADF)`, `Z6 (HFS)`).
    pub medium: blorb::medium::DiskImage,
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
    /// `medium` is required rather than optional on purpose: it is half of what
    /// names a Version 6 game's directory, and a caller that could omit it would
    /// silently produce a key that looks right and points at another machine's
    /// saves.
    pub fn of(bytes: &[u8], medium: blorb::medium::DiskImage) -> Option<DiskBuild> {
        let (version, release, serial) = DiskBuild::header_of(bytes)?;
        Some(DiskBuild { version, medium, release, serial })
    }

    /// The `(version, release, serial)` a story's header carries, for a caller
    /// that wants the story's IDENTITY without a medium to attach it to — the
    /// cross-volume fold in [`crate::disk_set`] is the one, and it folds on
    /// release and serial alone.
    pub fn header_of(bytes: &[u8]) -> Option<(u8, u16, String)> {
        if bytes.len() < 0x18 || !(3..=8).contains(&bytes[0]) {
            return None;
        }
        let serial: String = bytes[0x12..0x18].iter().map(|c| char::from(c & 0x7f)).collect();
        if !serial.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return None;
        }
        Some((bytes[0], u16::from_be_bytes([bytes[0x02], bytes[0x03]]), serial))
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
/// a person reading `~/.lanthorn` can tell `hitchhikers-guide-r59-s851108` from
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
    let base = format!("{}-r{}-s{serial}", slug_for(build.release, &build.serial), build.release);
    match build.version {
        // **A Version 6 game's saves are per-MEDIUM, because they are per-machine**
        // (SQ-1068). One build can be pressed onto several disks — *Arthur* release
        // 54 / serial 890606 is on both the Amiga floppy and the Macintosh
        // Masterpieces volume, byte for byte — and for v1-v5 that is one game with
        // one set of saves, which is what lets a key survive renaming an image or a
        // game moving between disks in a set (SQ-0850, and its guard still holds).
        //
        // Version 6 is where the two stop being interchangeable. A host Save State
        // swaps memory under a game that never learns it happened, and it carries
        // the v6 screen in NATIVE PIXELS plus the palette — so the Amiga's snapshot
        // restored into the Macintosh press puts a 640x400 screen into a 480x300
        // machine, with the wrong palette and a cell and face the story was never
        // told about. It looks plausible and is laid out for a screen the player
        // never sees, which is the SQ-0901 shape.
        // `reconcile_restored_screen_size` reconciles a restore into a different
        // PANE; there is nothing that reconciles one into a different MACHINE.
        //
        // The suffix is the medium's own label, the same token the story list shows
        // in its TYPE parenthetical, so a player reading `arthur-r54-s890606-hfs`
        // sees the row they launched.
        6 => format!("{base}-{}", build.medium.label().to_ascii_lowercase()),
        _ => base,
    }
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
    // `detect` first: mounting consumes the bytes, and the overwhelming majority
    // of calls are about an ordinary story file.
    let kind = blorb::medium::DiskImage::detect(&raw)?;
    // Across the SET, exactly as the launch path mounts (SQ-0952). This used to
    // be `MountedDisk::mount` — the platter alone — so a volume whose story comes
    // from the RELEASE rather than from itself found nothing, returned `None`,
    // and its caller fell back to the basename key while `startup.rs` keyed the
    // same game on its build. Two identities for one game, decided by which door
    // the caller came in.
    //
    // It shows on the story list. `picker::metadata_title` reads a story's
    // fetched metadata out of the directory this key names, and `startup.rs`
    // hands the in-game pane the build-keyed one (`metadata_title_in`) — whose
    // doc promises "the list and the pane cannot name the same game differently".
    // For a set-sourced volume they did, and which of the two could see the
    // metadata depended on which had fetched it.
    //
    // Costs sibling reads only when the platter yields nothing: `mount_at`
    // returns as soon as the named volume has a story of its own, which is every
    // loose file and every single-disk press.
    let disk = crate::disk_set::mount_at(story_path, raw).ok()?;
    let chosen = match disk_entry {
        Some(want) => disk
            .stories()
            .into_iter()
            .find(|s| s.name == want || s.name.eq_ignore_ascii_case(want))?,
        None => disk.story()?,
    };
    DiskBuild::of(&chosen.bytes, kind)
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

/// The extension a **Quetzal** save carries: the Z-machine's own standard save
/// format, which `gvm-cli` writes too because Glulx saves are Quetzal chunks in a
/// different wrapper.
///
/// Named rather than spelled out at each call site because it is a *claim about
/// the bytes*, not a filename convention — see [`SCOTT_EXT`].
pub const QUETZAL_EXT: &str = "qzl";

/// The extension a **Scott Adams** host snapshot carries (SQ-0919).
///
/// Not `.qzl`, and the difference is the whole reason these helpers take an
/// extension at all. Scott has no game-native save protocol and nothing Quetzal
/// about it: `scott::Vm::snapshot` writes item locations, flags, counters and the
/// lamp as little-endian words of its own. Calling that `cellar.qzl` would be a
/// lie about the format, and it would make a Scott save indistinguishable by name
/// from a Z-machine one if the two ever shared a directory.
pub const SCOTT_EXT: &str = "sav";

/// Resolve what the player typed at a save/restore filename prompt.
///
/// A bare name lands in `game_dir` with `ext`, so `quick` means this game's quick
/// save and not a file in whatever directory the shell happened to be in.
/// Anything carrying a path separator is honoured verbatim, which is the escape
/// hatch for saving somewhere else on purpose.
///
/// `ext` is the host's own — [`QUETZAL_EXT`] or [`SCOTT_EXT`] — and it travels
/// with every call because a save's extension states what its bytes ARE. It must
/// match whatever [`existing_saves`] was asked for, or the list and the resolver
/// stop being inverses.
pub fn resolve_save_input(input: &str, game_dir: &Path, ext: &str) -> PathBuf {
    let t = input.trim();
    if t.contains('/') || t.contains('\\') {
        return PathBuf::from(t);
    }
    let dotted = format!(".{ext}");
    // The typed extension is matched case-insensitively too, so `cellar.QZL` is
    // not turned into `cellar.QZL.qzl`.
    let has_ext = t.len() >= dotted.len()
        && t[t.len() - dotted.len()..].eq_ignore_ascii_case(&dotted);
    let name = if has_ext { t.to_string() } else { format!("{t}{dotted}") };
    // **An existing file wins, under the spelling it actually has on disk.**
    //
    // A case-insensitive filesystem hid a real defect here (SQ-0925).
    // `existing_saves` accepts `.QZL` as well as `.qzl`, so it lists `auto` for a
    // file called `auto.QZL`; macOS and Windows then open that file through the
    // `auto.qzl` spelling built above, and Linux does not. The list offered a save
    // the restore could not open, on one platform only.
    //
    // The scan comes BEFORE the exact check rather than after it, so every platform
    // returns the same path for the same directory. Resolving to `auto.qzl` on a
    // filesystem that quietly matches it and to `auto.QZL` on one that does not
    // would fix the bug and leave a path that reads differently in a prompt, an
    // error message and an overwrite warning depending on the host.
    //
    // A NEW name matches nothing and falls through to the exact spelling, which is
    // what saving wants.
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().eq_ignore_ascii_case(&name) {
                return e.path();
            }
        }
    }
    game_dir.join(&name)
}

/// Every save already in this game's directory, named the way you would type it.
///
/// The exact inverse of [`resolve_save_input`] **for the same `ext`**: that turns
/// `cellar` into `<game_dir>/cellar.qzl`, this turns the directory back into
/// `["cellar", …]`. Sorted, so the numbering a prompt shows is stable between
/// turns — a list that reorders itself under the player is worse than no list
/// (SQ-0918).
///
/// Passing a different `ext` than the resolver was given breaks that inverse and
/// is the one way to misuse this pair: the list would offer names the restore
/// cannot open, which is exactly the shape of the bug SQ-0925 fixed.
///
/// Case-insensitive on the extension, because `resolve_save_input` accepts a typed
/// `.QZL` and a directory can be copied from a case-preserving filesystem. A
/// directory that does not exist yet — the ordinary state before the first save —
/// is empty rather than an error.
pub fn existing_saves(game_dir: &Path, ext: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(game_dir) else {
        return Vec::new();
    };
    let dotted = format!(".{ext}");
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.get(..name.len().checked_sub(dotted.len())?)?;
            name[stem.len()..].eq_ignore_ascii_case(&dotted).then(|| stem.to_string())
        })
        .filter(|stem| !stem.is_empty())
        .collect();
    out.sort_by_key(|n| n.to_lowercase());
    out
}

/// Resolve what the player typed at a save/restore prompt against a numbered list.
///
/// `Some(name)` when the input is a 1-based index into `saves`; `None` for anything
/// else, which the caller then treats as a filename exactly as before. So a save
/// legitimately CALLED `2` is still reachable — as `2.qzl`, or with a path — and a
/// list that is empty can never capture an input.
///
/// Deliberately not applied at a SAVE prompt: there a number would mean "overwrite
/// that one", and silently clobbering a save the player named is the defect SQ-0648
/// fixed in the TUI. The list is shown there as a reminder of what you would collide
/// with, not as a way to pick a target.
pub fn pick_save<'a>(input: &str, saves: &'a [String]) -> Option<&'a str> {
    let n: usize = input.trim().parse().ok()?;
    saves.get(n.checked_sub(1)?).map(String::as_str)
}

/// The one-line reminder a save/restore prompt prints above itself, or `None` when
/// there is nothing saved yet — in which case a first save looks exactly as it
/// always did.
pub fn save_list_line(saves: &[String]) -> Option<String> {
    (!saves.is_empty()).then(|| {
        let items: Vec<String> =
            saves.iter().enumerate().map(|(i, n)| format!("{} {n}", i + 1)).collect();
        format!("saves: {}", items.join("   "))
    })
}

/// The confirmation a save prompt owes the player when the name they typed already
/// exists, or `None` when it does not (SQ-0918).
///
/// `fs::write` is unconditional, so both CLIs silently clobbered — the same defect
/// SQ-0648 fixed in the TUI, which prompts and defaults to Cancel. Naming the
/// EXISTING save rather than echoing what was just typed is deliberate there and
/// here: the two can differ, and what the player needs to know is what they are
/// about to lose.
///
/// The reading is left to the caller because the three hosts read input in three
/// different ways — cooked stdin, a raw-mode editor, a Glk line request — and this
/// module has no business picking one.
pub fn overwrite_warning(path: &Path) -> Option<String> {
    path.is_file().then(|| {
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        format!("'{name}' already exists. Overwrite? (y/N) ")
    })
}

/// Whether an answer to [`overwrite_warning`] is a yes.
///
/// Anything else — including a bare Enter and EOF — is a no, so the destructive
/// branch is never the one you reach by not answering. That matches the TUI's
/// dialog, which opens with Cancel focused.
pub fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── existing_saves / pick_save / save_list_line (SQ-0918) ───────────────

    /// A scratch directory holding `names`, on the same
    /// pid+thread pattern `the_game_dir_can_actually_be_created_beside_the_story_file`
    /// uses — nextest gives every test its own process, but `cargo test` does not, so
    /// the thread id is what keeps two of these apart.
    fn dir_with(tag: &str, names: &[&str]) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "cli-host-saves-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for n in names {
            std::fs::write(d.join(n), b"x").unwrap();
        }
        d
    }

    /// SQ-0919: **two hosts, two extensions, and neither sees the other's saves.**
    ///
    /// This is why the extension is a parameter rather than a constant inside these
    /// two functions. A Scott snapshot is not Quetzal, so calling it `.qzl` would be
    /// a lie about the bytes — and worse, `existing_saves` would then list a save
    /// the other host's restore cannot read. The pair are inverses only for the
    /// same `ext`, and this is the case that says so.
    #[test]
    fn each_hosts_extension_lists_and_resolves_only_its_own() {
        let d = dir_with("exts", &["cellar.qzl", "cellar.sav", "notes.txt"]);

        assert_eq!(existing_saves(&d, QUETZAL_EXT), vec!["cellar".to_string()]);
        assert_eq!(existing_saves(&d, SCOTT_EXT), vec!["cellar".to_string()]);

        // Same typed name, two different files — which is the whole point.
        let q = resolve_save_input("cellar", &d, QUETZAL_EXT);
        let sc = resolve_save_input("cellar", &d, SCOTT_EXT);
        assert_ne!(q, sc);
        assert_eq!(q.extension().unwrap(), "qzl");
        assert_eq!(sc.extension().unwrap(), "sav");

        // And a name typed WITH the other host's extension is not stripped of it:
        // `cellar.qzl` asked of the Scott host is a file called `cellar.qzl.sav`,
        // not a Quetzal save it has no business opening.
        assert_eq!(
            resolve_save_input("cellar.qzl", &d, SCOTT_EXT).file_name().unwrap(),
            "cellar.qzl.sav",
        );

        // The inverse holds within each extension, which is the property SQ-0925
        // is about and it must survive the parameterisation.
        for ext in [QUETZAL_EXT, SCOTT_EXT] {
            for name in existing_saves(&d, ext) {
                assert!(resolve_save_input(&name, &d, ext).is_file(), "{name}.{ext} round-trips");
            }
        }
    }

    /// The inverse of `resolve_save_input`, and only that: `.qzl` files, named the
    /// way you would type them back.
    #[test]
    fn existing_saves_lists_what_resolve_save_input_would_have_written() {
        let d = dir_with("mixed", &["cellar.qzl", "before-maze.qzl", "notes.txt", "auto.QZL"]);
        assert_eq!(
            existing_saves(&d, QUETZAL_EXT),
            vec!["auto".to_string(), "before-maze".to_string(), "cellar".to_string()],
            "sorted case-insensitively, extension stripped, .txt ignored, .QZL kept",
        );
        // And it really is the inverse. **Assert the resolved FILENAME, not just
        // that something is there**: `is_file()` alone passes on macOS and Windows
        // for the wrong reason, because their filesystems match `auto.qzl` to
        // `auto.QZL` themselves, and it was failing on Linux for the right one
        // (SQ-0925).
        assert_eq!(
            resolve_save_input("auto", &d, QUETZAL_EXT).file_name().unwrap(),
            "auto.QZL",
            "the list offered `auto`, so it must resolve to the file that produced it",
        );
        for name in existing_saves(&d, QUETZAL_EXT) {
            assert!(resolve_save_input(&name, &d, QUETZAL_EXT).is_file(), "{name} round-trips");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A directory that does not exist yet is the ordinary state before the first
    /// save, not an error.
    #[test]
    fn a_game_with_no_saves_yet_lists_nothing_and_prints_nothing() {
        let missing = std::path::Path::new("/no/such/game/dir/anywhere");
        assert!(existing_saves(missing, QUETZAL_EXT).is_empty());
        assert_eq!(save_list_line(&[]), None, "a first save looks exactly as it always did");
    }

    /// A number picks from the list; anything else is a filename, as before.
    #[test]
    fn a_number_picks_from_the_list_and_nothing_else_does() {
        let saves = vec!["cellar".to_string(), "troll".to_string()];
        assert_eq!(pick_save("1", &saves), Some("cellar"));
        assert_eq!(pick_save("  2  ", &saves), Some("troll"), "whitespace is trimmed");
        assert_eq!(pick_save("3", &saves), None, "past the end is a filename");
        assert_eq!(pick_save("0", &saves), None, "the list is 1-based");
        assert_eq!(pick_save("cellar", &saves), None, "a name is a name");
        assert_eq!(pick_save("2x", &saves), None);
        assert_eq!(pick_save("", &saves), None, "empty is a cancel, not a pick");
        assert_eq!(pick_save("1", &[]), None, "an empty list can never capture an input");
    }

    /// A save legitimately CALLED `2` stays reachable, which is why `pick_save` is
    /// consulted rather than the filename path being replaced.
    #[test]
    fn a_save_named_like_a_number_is_still_reachable() {
        let d = dir_with("numeric", &["2.qzl", "cellar.qzl"]);
        let saves = existing_saves(&d, QUETZAL_EXT);
        assert_eq!(saves, vec!["2".to_string(), "cellar".to_string()]);
        // Typing `2` picks the FIRST entry, which here happens to be the file `2`.
        assert_eq!(pick_save("2", &saves), Some("cellar"), "the index wins at the prompt");
        // …and the file itself is still addressable by its full name.
        assert!(resolve_save_input("2.qzl", &d, QUETZAL_EXT).is_file(), "and by name with the extension");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_prompt_line_numbers_them_from_one() {
        let line = save_list_line(&["cellar".to_string(), "troll".to_string()]).expect("some");
        assert!(line.starts_with("saves: "), "{line:?}");
        assert!(line.contains("1 cellar"), "{line:?}");
        assert!(line.contains("2 troll"), "{line:?}");
    }

    /// A name that exists warns; one that does not goes straight through.
    #[test]
    fn an_existing_save_warns_and_names_itself() {
        let d = dir_with("overwrite", &["cellar.qzl"]);
        let warn = overwrite_warning(&resolve_save_input("cellar", &d, QUETZAL_EXT)).expect("exists");
        assert!(warn.contains("'cellar'"), "names the save that would be lost: {warn:?}");
        assert!(warn.contains("(y/N)"), "and shows which answer is the safe one: {warn:?}");
        assert_eq!(overwrite_warning(&resolve_save_input("troll", &d, QUETZAL_EXT)), None, "a new name");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Only an explicit yes is a yes — the destructive branch must not be the one
    /// you reach by not answering.
    #[test]
    fn only_an_explicit_yes_overwrites() {
        assert!(is_yes("y") && is_yes("Y") && is_yes("yes") && is_yes("  YES  "));
        for no in ["", " ", "n", "no", "cellar", "yep", "1"] {
            assert!(!is_yes(no), "{no:?} is not a yes");
        }
    }

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

    /// A v5 build off an Amiga floppy — the medium-agnostic case, so the keys the
    /// cases below assert are the bare `<slug>-r<release>-s<serial>` form.
    fn build(release: u16, serial: &str) -> DiskBuild {
        build_v(5, release, serial, blorb::medium::DiskImage::Adf)
    }

    fn build_v(
        version: u8,
        release: u16,
        serial: &str,
        medium: blorb::medium::DiskImage,
    ) -> DiskBuild {
        DiskBuild { version, medium, release, serial: serial.to_string() }
    }

    /// **A Version 6 game's key names its MEDIUM; a v1-v5 game's does not**
    /// (SQ-1068).
    ///
    /// *Arthur* release 54 / serial 890606 is one build pressed onto two disks —
    /// the Amiga floppy and the Macintosh *Masterpieces* volume — so before this
    /// they shared a drawer, and an auto-save made on one was silently resumed by
    /// the other. For v1-v5 that sharing is the intended behaviour and
    /// [`one_build_across_three_media_resolves_to_one_directory`] still pins it:
    /// a save is VM memory and says nothing about the machine.
    ///
    /// Version 6 is where the machine stops being incidental. The archive carries
    /// the screen in NATIVE PIXELS plus its palette, so the Amiga's 640x400
    /// snapshot restored into the Macintosh's 480x300 press is laid out for a
    /// screen the player never sees.
    #[test]
    fn a_version_six_key_names_its_medium_and_a_v5_key_does_not() {
        use blorb::medium::DiskImage;
        // The reported collision: one build, two media, two machines.
        assert_eq!(
            disk_story_key(&build_v(6, 54, "890606", DiskImage::Adf)),
            "arthur-r54-s890606-adf",
        );
        assert_eq!(
            disk_story_key(&build_v(6, 54, "890606", DiskImage::Hfs)),
            "arthur-r54-s890606-hfs",
        );
        // …and the same build at v5 keeps one drawer whatever it is pressed on,
        // which is the property SQ-0850 established and this must not disturb.
        for medium in [DiskImage::Adf, DiskImage::Hfs, DiskImage::Fat12Dos] {
            assert_eq!(
                disk_story_key(&build_v(5, 88, "840726", medium)),
                "zork-i-r88-s840726",
                "a v5 build is medium-agnostic, on {medium:?}",
            );
        }
        // The suffix is the medium's own label — the token the story list shows
        // in its TYPE parenthetical — so the directory names the row it came from.
        assert_eq!(DiskImage::Adf.label().to_ascii_lowercase(), "adf");
        assert_eq!(DiskImage::Hfs.label().to_ascii_lowercase(), "hfs");
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
        assert_eq!(DiskBuild::of(b"Glul\0\0\0\0", blorb::medium::DiskImage::Adf), None, "Glulx magic, not a Z header");
        assert_eq!(DiskBuild::of(&[], blorb::medium::DiskImage::Adf), None);
        assert_eq!(DiskBuild::of(&header(2, 88, "840726"), blorb::medium::DiskImage::Adf), None, "v2 is out of range");
        assert_eq!(DiskBuild::of(&header(9, 88, "840726"), blorb::medium::DiskImage::Adf), None, "v9 is out of range");
        assert_eq!(
            DiskBuild::of(&header(3, 88, "840726"), blorb::medium::DiskImage::Adf),
            Some(build_v(3, 88, "840726", blorb::medium::DiskImage::Adf)),
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
        assert_eq!(
            DiskBuild::of(&bytes, blorb::medium::DiskImage::Adf),
            Some(build_v(3, 0, "Blown!", blorb::medium::DiskImage::Adf)),
        );
        // No title in the table answers to release 0, so it slugs as `story`,
        // and `!` is not a directory character.
        assert_eq!(disk_story_key(&build(0, "Blown!")), "story-r0-sBlown_");
        // Binary is still binary with the bit masked: a saved game does not key.
        let mut save = header(3, 0, "......");
        save[0x12..0x18].copy_from_slice(&[0x00; 6]);
        assert_eq!(DiskBuild::of(&save, blorb::medium::DiskImage::Adf), None, "an all-zero serial is not text");
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
        assert_eq!(resolve_save_input("quick", gd, QUETZAL_EXT), PathBuf::from("/data/Zork1.z5.save/quick.qzl"));
        assert_eq!(
            resolve_save_input("quick.qzl", gd, QUETZAL_EXT),
            PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "an extension the player typed is not doubled"
        );
        assert_eq!(resolve_save_input("/tmp/foo.qzl", gd, QUETZAL_EXT), PathBuf::from("/tmp/foo.qzl"));
        // A RELATIVE path is the case that actually pins the rule: `Path::join`
        // with an absolute path replaces the whole thing, so the absolute case
        // above passes either way and cannot tell a working escape hatch from a
        // broken one.
        assert_eq!(
            resolve_save_input("saves/foo.qzl", gd, QUETZAL_EXT),
            PathBuf::from("saves/foo.qzl"),
            "a path the player typed is honoured, not reparented into the game dir"
        );
        assert_eq!(resolve_save_input("  quick  ", gd, QUETZAL_EXT), PathBuf::from("/data/Zork1.z5.save/quick.qzl"),
            "trimmed, because the prompt line carries whatever was typed");
    }
}
