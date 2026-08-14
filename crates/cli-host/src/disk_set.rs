//! **A multi-disk release is one collection, not several disks** (SQ-0844).
//!
//! *The Lost Treasures of Infocom* ships as seven Apple II volumes, the Atari ST
//! compilations as nine, the DOS press as `floppy1.ima`…`floppy5.ima`. Every one
//! of those is a shelf of games that happens to have been cut across media, and
//! the browser had no idea: naming `disk1.img` opened whatever single story that
//! one image offered, and the twenty-nine games on its siblings were reachable
//! only by naming each sibling in turn.
//!
//! This module answers exactly one question — **which files are volumes of one
//! set** — from their names. What the answer is *used* for lives elsewhere:
//! `app::picker::StorySource` scans a set, `app::picker::scan_stories` folds a
//! set's duplicate builds together, and [`mount_at`] hands the set to
//! [`blorb::medium::MountedDisk::mount_set`] for both front-ends.
//!
//! # Why it is here and not in `app` (SQ-0874)
//!
//! It lived in `app` until the CLI needed it, and the CLI cannot depend on `app`.
//! `zvm-cli` opened a disk with [`blorb::medium::MountedDisk::mount`] — that is
//! `mount_set` with no companions — so **no multi-volume release opened in the
//! CLI at all**: *Trinity* played in the TUI and not at the prompt, and the
//! Apple II presses reported "no story file on this disk image" off a disk whose
//! game is simply on the next floppy.
//!
//! The alternative was a second copy of the rule in `zvm-cli`, which is exactly
//! what [`blorb::medium`]'s module doc exists to prevent: two front-ends with two
//! ideas of what a release is disagree eventually, and the disagreement is
//! invisible until a game goes missing from one of them. `cli-host` is the crate
//! both front-ends already share and it already depends on `blorb`, so the rule
//! moved down rather than being copied sideways — the same trade SQ-0850 made for
//! the per-game save key. `app::disk_set` is now a re-export of this module, so
//! there is one implementation and every existing call site still spells it the
//! way it always did.
//!
//! # The rule
//!
//! Two files are volumes of one set when they sit in one directory, share a
//! **disk-image extension**, and their stems are identical except at **one run
//! of decimal digits** whose values across the group are distinct, include `1`,
//! and are none of them greater than [`MAX_INDEX`]. Exactly one digit run may
//! qualify; a stem where two of them do forms no set at all.
//!
//! # What it refuses, and why each one matters
//!
//! Recognition is deliberately conservative, because the two failure modes are
//! not symmetric. Missing a set costs the player a menu they can rebuild by
//! naming the other disks. A *false* set silently folds unrelated games into one
//! collection and, through the IFID dedupe, can hide a row. So:
//!
//! - **`adv01.dat` … `adv13.dat`** — thirteen unrelated Scott Adams games, and a
//!   textbook prefix-plus-index. `.dat` is not a disk-image extension, so they
//!   are thirteen games and never a set. This is the case that makes the
//!   extension test load-bearing rather than decorative.
//! - **Zork Zero's `(360K)` and `(720K)` DOS presses.** Both spell their disks
//!   `(Disk 1)`, `(Disk 2)`, so `360K (Disk 1)` and `720K (Disk 1)` differ at
//!   exactly one digit run — and that run's values are `{360, 720}`, which
//!   contain no `1` and exceed [`MAX_INDEX`]. Refused, and the two presses stay
//!   the two separate three- and two-disk sets they are. This is the corpus's
//!   sharpest false positive and the reason the index test is not merely
//!   "the values differ".
//! - **Years.** `(1993)`, `(1989)` and the `19` of `(19xx)` are constant within
//!   their sets so they never even vary; a run that varied only in years would
//!   fail the same `1`-and-[`MAX_INDEX`] test that stops `{360, 720}`.
//! - **`disk*.img` against `floppy*.ima`.** Different stem text *and* different
//!   extension: two sets, which is what they are.
//! - **Roman numerals** — `Zork I/II/III…adf` carry no digits at all.
//! - **A set whose disk 1 is absent.** Requiring the index to reach `1` is what
//!   makes `{360, 720}` fall over, and it is honest about the cost: hand this
//!   `floppy2.ima`…`floppy5.ima` with `floppy1.ima` deleted and it reports no
//!   set. The games are all still listed by an ordinary directory scan; only the
//!   dedupe and the launch-from-one-member menu are lost.
//!
//! # What it accepted the day a format learned a spelling
//!
//! **`shogun_s1.dsk`…`s5` and `zork_zero_1.dsk`…`4`.** These were refused for
//! one reason only — `.dsk` was not a spelling [`blorb::medium`] claimed.
//! SQ-0864 gave it to the ProDOS row (a 5.25-inch dump is a ProDOS volume with
//! its sectors in the drive's order), and **this module needed no change at
//! all**: the extension census is read off that table, so the two presses became
//! sets the same day the reader landed. That is the whole argument for
//! [`has_image_ext`] asking `blorb::medium` rather than keeping a list, and it
//! is now a measured one rather than a stated one.
//!
//! It also matters more here than for any set before it. The compilations are a
//! convenience — miss one and the games are still listed, disk by disk. These
//! two releases page **one story across every volume of the set**, so a set that
//! is not recognised is not an inconvenience; it is a game that cannot be
//! opened at all.
//!
//! # Why the filename and not the volume label
//!
//! Every mountable format here *could* be asked its volume name, and on the
//! Apple II press the answer is beautiful — `INFOCOM1`…`INFOCOM7`, inside the
//! disk where no rename can reach it. It is still the wrong signal, on measured
//! grounds:
//!
//! - **Nine of the corpus's volumes have no label at all.** Every
//!   `Infocom Compilation N (19xx)(-).st` reports `volume_name() == None`, so a
//!   label rule would cover the Apple II and DOS families and leave the entire
//!   Atari ST shelf — 39 of the 100 rows — ungrouped.
//! - **Labels are not unique.** Zork Zero's 360K and 720K DOS presses both label
//!   their first disk `ZORK0 1` and their second `ZORK0 2`. Grouping on the label
//!   merges the two presses, which is precisely the false positive the filename
//!   rule refuses.
//! - **It would cost a mount.** The name is the one thing already in hand.
//!
//! So the label is not consulted. It is a good corroborating signal and a bad
//! primary one, and a second signal that only sometimes applies is a second
//! answer waiting to disagree with the first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The largest disk number a set may carry. Real multi-disk releases are small —
/// the biggest here is nine — and the bound is what tells a *disk index* apart
/// from a year, a release number or a `360K`/`720K` capacity that happens to sit
/// in the same place in two filenames.
pub const MAX_INDEX: u64 = 32;

/// One piece of a stem: a run of decimal digits, or the literal text between
/// runs.
///
/// A numeric run keeps its **raw spelling** as well as its value. Constant runs
/// are compared as written, so `Disk 1 of 07` and `Disk 2 of 7` are not volumes
/// of one set, and a run too long to be a number cannot collide with a short one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Token {
    Text(String),
    /// The digits exactly as spelled.
    Num(String),
}

impl Token {
    /// A numeric run's value, or `None` when it is text or too long to be one.
    fn value(&self) -> Option<u64> {
        match self {
            Token::Text(_) => None,
            Token::Num(d) => d.parse().ok(),
        }
    }
}

/// Split a stem into alternating text and digit runs.
fn tokens(stem: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut digits = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_digit() {
            if !text.is_empty() {
                out.push(Token::Text(std::mem::take(&mut text)));
            }
            digits.push(ch);
        } else {
            if !digits.is_empty() {
                out.push(Token::Num(std::mem::take(&mut digits)));
            }
            text.push(ch);
        }
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    if !digits.is_empty() {
        out.push(Token::Num(digits));
    }
    out
}

/// Is this a spelling [`blorb::medium`] gives one of its disk formats?
///
/// A pre-filter and never evidence of a format — exactly as
/// [`blorb::medium::DiskImage::extensions`] requires. Here it is doing a
/// different job from the picker's: it is what keeps a numbered run of ordinary
/// **story files** from being read as a run of disks.
fn has_image_ext(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    blorb::medium::image_extensions().any(|e| e == ext)
}

/// The stem and lowercased extension a grouping key is built from.
fn parts(path: &Path) -> Option<(Vec<Token>, String)> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let ext = path.extension().and_then(|e| e.to_str())?.to_ascii_lowercase();
    Some((tokens(stem), ext))
}

/// Do these index values describe a run of **disks**?
///
/// Distinct, reaching `1`, and none above [`MAX_INDEX`]. See the module docs for
/// what each clause is holding back.
fn is_index_run(values: &[u64]) -> bool {
    if values.len() < 2 {
        return false;
    }
    let mut seen: Vec<u64> = values.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len() == values.len()
        && seen.first() == Some(&1)
        && seen.last().is_some_and(|&m| m <= MAX_INDEX)
}

/// Partition `files` into multi-disk sets, each **ordered by disk number**.
///
/// Only sets are returned: a file in no set appears in no group, and a group
/// always holds at least two files. Deterministic for a given input, and
/// independent of the order `files` arrives in — the caller may hand over a
/// directory listing in whatever order the filesystem produced.
pub fn group(files: &[PathBuf]) -> Vec<Vec<PathBuf>> {
    group_indexed(files)
        .into_iter()
        .map(|g| g.into_iter().map(|(_, p)| p).collect())
        .collect()
}

/// [`group`], keeping each volume's **disk number** — the value of the digit run
/// that varies across the set (SQ-0865).
///
/// The number is the one the release itself spells, not the position in the
/// list: `is_index_run` requires the values to be distinct, to reach `1` and to
/// stay under [`MAX_INDEX`], but never to be contiguous, so a set missing its
/// middle platter still reports `3` for the disk labelled 3. That is the whole
/// reason this is exposed rather than left to a caller counting positions — a
/// dialog that says "from disk 2" about disk 3 is worse than one that says
/// nothing.
pub fn group_indexed(files: &[PathBuf]) -> Vec<Vec<(u64, PathBuf)>> {
    // Candidates: disk-image spellings only, sorted so the result never depends
    // on `read_dir` order.
    let mut cands: Vec<&PathBuf> = files.iter().filter(|p| has_image_ext(p)).collect();
    cands.sort();
    cands.dedup();

    // Bucket by "everything except digit run `pos`". A file with three digit runs
    // is offered under three keys; a key collects the files that agree on all the
    // OTHER runs, which is exactly the candidate set for varying that one.
    type Key = (String, usize, Vec<Token>);
    let mut buckets: HashMap<Key, Vec<(u64, PathBuf)>> = HashMap::new();
    for path in &cands {
        let Some((toks, ext)) = parts(path) else { continue };
        for (pos, tok) in toks.iter().enumerate() {
            let Some(value) = tok.value() else { continue };
            let mut holed = toks.clone();
            // The hole must not be confusable with any literal text.
            holed[pos] = Token::Text("\u{0}".to_string());
            buckets
                .entry((ext.clone(), pos, holed))
                .or_default()
                .push((value, (*path).clone()));
        }
    }

    // Keep the buckets whose varying run really is a disk index.
    let mut groups: Vec<Vec<(u64, PathBuf)>> = buckets
        .into_values()
        .filter(|members| {
            let values: Vec<u64> = members.iter().map(|(v, _)| *v).collect();
            is_index_run(&values)
        })
        .collect();

    // Ambiguity is refused, not resolved: a file that two different digit runs
    // each make a plausible set out of has no unambiguous set, and guessing which
    // is the disk number is exactly the false positive this module exists to
    // avoid. Both candidate groups go.
    let mut times_seen: HashMap<PathBuf, usize> = HashMap::new();
    for g in &groups {
        for (_, p) in g {
            *times_seen.entry(p.clone()).or_default() += 1;
        }
    }
    groups.retain(|g| g.iter().all(|(_, p)| times_seen.get(p) == Some(&1)));

    let mut out: Vec<Vec<(u64, PathBuf)>> = groups
        .into_iter()
        .map(|mut g| {
            g.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
            g
        })
        .collect();
    // Ordered by the paths alone, exactly as the `Vec<Vec<PathBuf>>` this used to
    // build sorted itself — the disk numbers must not become a second sort key.
    out.sort_by(|a, b| a.iter().map(|(_, p)| p).cmp(b.iter().map(|(_, p)| p)));
    out
}

/// Every volume of the set `path` belongs to — `path` included — in disk order,
/// or `None` when it is not part of one.
///
/// Reads `path`'s directory and nothing else: the rule is entirely a question
/// about names, so this never opens a disk.
pub fn members(path: &Path) -> Option<Vec<PathBuf>> {
    Some(members_indexed(path)?.into_iter().map(|(_, p)| p).collect())
}

/// [`members`], each volume paired with the **disk number** its name carries
/// (SQ-0865).
pub fn members_indexed(path: &Path) -> Option<Vec<(u64, PathBuf)>> {
    if !has_image_ext(path) {
        return None;
    }
    let dir = path.parent()?;
    let files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    group_indexed(&files).into_iter().find(|g| g.iter().any(|(_, m)| m == path))
}

/// Open the disk image `path`, whose bytes are `raw`, with the other volumes of
/// its multi-disk release available to it (SQ-0864).
///
/// **The one way a front-end mounts a named disk**, and the reason it is one way:
/// a story can live on no single floppy. The Apple II 5.25-inch presses of
/// *Shogun*, *Journey* and *Zork Zero* page one game across five, five and four
/// volumes, and the Commodore *Trinity* pages one across two sides — so opening
/// any one of them and asking what is on it is a question only the whole release
/// can answer.
///
/// Which files are one release is [`members`]'s answer, from their names and
/// without opening anything. It is deliberately not `blorb`'s: naming is
/// filesystem policy and that crate is given bytes.
///
/// **Nothing is read eagerly.** [`blorb::medium::MountedDisk::mount_set`] calls
/// the closure only when the named volume turns out to have no story of its own,
/// so an ordinary floppy and every volume of a compilation cost exactly the one
/// read they always did — which is what keeps a library scan from reading seven
/// 800 KB floppies per row.
pub fn mount_at(
    path: &Path,
    raw: Vec<u8>,
) -> Result<blorb::medium::MountedDisk, blorb::medium::MountError> {
    blorb::medium::MountedDisk::mount_set(raw, || {
        members(path)
            .unwrap_or_default()
            .into_iter()
            .filter(|m| m != path)
            .filter_map(|m| std::fs::read(m).ok())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(|n| PathBuf::from("/stories").join(n)).collect()
    }

    fn names_of(g: &[PathBuf]) -> Vec<String> {
        g.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// The set the user named: `diskN.img`.
    #[test]
    fn a_trailing_index_on_a_disk_image_is_a_set() {
        let g = group(&paths(&["disk1.img", "disk2.img", "disk3.img", "disk4.img"]));
        assert_eq!(g.len(), 1);
        assert_eq!(names_of(&g[0]), ["disk1.img", "disk2.img", "disk3.img", "disk4.img"]);
    }

    /// …and it is ordered by disk number, not by filename, so a ten-disk set does
    /// not run 1, 10, 2.
    #[test]
    fn members_are_ordered_by_disk_number() {
        let g = group(&paths(&["d10.img", "d2.img", "d1.img"]));
        assert_eq!(names_of(&g[0]), ["d1.img", "d2.img", "d10.img"]);
    }

    /// Two families in one directory are two sets — the stem text and the
    /// extension both differ, and either alone would be enough.
    #[test]
    fn dos_floppies_and_dos_disks_are_two_sets() {
        let g = group(&paths(&["disk1.img", "disk2.img", "floppy1.ima", "floppy2.ima"]));
        assert_eq!(g.len(), 2, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        let all: Vec<Vec<String>> = g.iter().map(|x| names_of(x)).collect();
        assert!(all.contains(&vec!["disk1.img".to_string(), "disk2.img".to_string()]));
        assert!(all.contains(&vec!["floppy1.ima".to_string(), "floppy2.ima".to_string()]));
    }

    /// A constant digit run elsewhere in the stem is no obstacle: the index is
    /// the run that varies, wherever it sits.
    #[test]
    fn an_index_in_the_middle_of_a_stem_is_found() {
        let g = group(&paths(&[
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 1 of 7).2mg",
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 2 of 7).2mg",
            "Lost Treasures of Infocom, The (1993)(BRCC)(Disk 3 of 7).2mg",
        ]));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].len(), 3);
        // The `1993` and the `of 7` never varied, so neither was ever a candidate.
    }

    #[test]
    fn the_atari_st_compilations_are_one_set() {
        let names: Vec<String> =
            (1..=9).map(|n| format!("Infocom Compilation {n} (19xx)(-).st")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let g = group(&paths(&refs));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].len(), 9, "one shelf of nine, not three shelves of three");
    }

    // ── What it refuses ──────────────────────────────────────────────────────

    /// **The false positive that matters.** Thirteen Scott Adams games named
    /// `advNN.dat` are a textbook prefix-plus-index and must never be a set,
    /// because merging them would put twelve unrelated games behind one of them.
    #[test]
    fn a_numbered_run_of_story_files_is_not_a_set() {
        let names: Vec<String> = (1..=13).map(|n| format!("adv{n:02}.dat")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        assert!(group(&paths(&refs)).is_empty(), ".dat is not a disk-image spelling");
    }

    /// **The free consequence** (SQ-0864). The Apple II 5.25-inch presses were
    /// refused here for exactly one reason — `.dsk` was not a spelling
    /// `blorb::medium` claimed — and the moment the ProDOS row claimed it they
    /// became sets, with not a line of this module changed.
    ///
    /// It is the sharpest case in the corpus for grouping at all: each of these
    /// floppies carries a fifth or a quarter of one story and nothing else, so a
    /// set that is not recognised is a game that cannot be opened. (*Journey*'s
    /// five joined *Shogun*'s and *Zork Zero*'s in SQ-0863, again with not a
    /// line of this module changed.)
    #[test]
    fn the_apple_five_and_a_quarter_inch_presses_are_two_sets() {
        let mut names: Vec<String> = (1..=5).map(|n| format!("shogun_s{n}.dsk")).collect();
        names.extend((1..=4).map(|n| format!("zork_zero_{n}.dsk")));
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let g = group(&paths(&refs));
        assert_eq!(g.len(), 2, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = g.iter().map(|x| x.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, [4, 5], "Zork Zero on four floppies, Shogun on five");
        // …and in disk order, which is the order the segments pair in.
        let shogun = g.iter().find(|x| x.len() == 5).expect("the five-disk set");
        assert_eq!(names_of(shogun), [
            "shogun_s1.dsk",
            "shogun_s2.dsk",
            "shogun_s3.dsk",
            "shogun_s4.dsk",
            "shogun_s5.dsk",
        ]);
    }

    /// **The free consequence, a second time** (SQ-0869) — and the first set in
    /// the corpus whose members SHOUT their extension.
    ///
    /// `TRINITY1.D64` and `TRINITY2.D64` are the two sides of Infocom's
    /// Commodore *Trinity*, a Version 4 story of 262,064 bytes on a pair of
    /// 174,848-byte floppies. They became a set the moment `blorb::medium`
    /// claimed `.d64`, with not a line of this module changed — including the
    /// case-folding, since `parts` lowercases and the census is lowercase.
    ///
    /// **And the lone *Hitchhiker's* disk is not a set**, which is the other
    /// half and the one worth asserting: its stem carries a four-digit year, so
    /// a rule that grouped on "there is a number in the name" would have made a
    /// one-member set out of it. `is_index_run` refuses a run of one and refuses
    /// values that do not start at 1, and 1984 is both.
    #[test]
    fn the_two_sides_of_the_commodore_trinity_are_one_set_and_hitchhikers_is_none() {
        let lone = "Hitchhikers_Guide_to_the_Galaxy_The_1984_Infocom.d64";
        let g = group(&paths(&["TRINITY1.D64", "TRINITY2.D64", lone]));
        assert_eq!(g.len(), 1, "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
        assert_eq!(names_of(&g[0]), ["TRINITY1.D64", "TRINITY2.D64"], "in side order");
        assert!(
            !g[0].iter().any(|p| p.ends_with(lone)),
            "a single disk with a year in its name is not a set"
        );
        // …and on its own it forms no group at all, rather than a group of one.
        assert!(group(&paths(&[lone])).is_empty());
    }

    /// …and the census is genuinely where that came from: nothing in this
    /// module names a spelling, so `.dsk` and `.d64` had to arrive through the
    /// table.
    #[test]
    fn the_extension_census_is_the_tables_and_not_this_modules() {
        assert!(has_image_ext(Path::new("/stories/shogun_s1.dsk")));
        assert!(blorb::medium::image_extensions().any(|e| e == "dsk"));
        assert!(has_image_ext(Path::new("/stories/TRINITY1.D64")), "and case does not decide it");
        assert!(blorb::medium::image_extensions().any(|e| e == "d64"));
        assert!(!has_image_ext(Path::new("/stories/adv01.dat")));
    }

    /// **The corpus's sharpest case.** Zork Zero's two DOS presses spell their
    /// disks alike, so `(360K) (Disk 1)` and `(720K) (Disk 1)` differ at exactly
    /// one digit run — and `{360, 720}` is a capacity, not a disk index. The two
    /// presses must come out as two sets of three and two.
    #[test]
    fn two_presses_of_one_game_are_two_sets() {
        let g = group(&paths(&[
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 1) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 2) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (360K) (Disk 3) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (720K) (Disk 1) [!].ima",
            "Zork Zero (1989) (r393, Serial 890714) (720K) (Disk 2) [!].ima",
        ]));
        assert_eq!(g.len(), 2, "the capacity run must not group the two presses");
        let sizes: Vec<usize> = {
            let mut s: Vec<usize> = g.iter().map(|x| x.len()).collect();
            s.sort_unstable();
            s
        };
        assert_eq!(sizes, [2, 3]);
        for set in &g {
            let caps: Vec<bool> =
                set.iter().map(|p| p.to_string_lossy().contains("360K")).collect();
            assert!(caps.iter().all(|c| *c == caps[0]), "a set mixed the two capacities");
        }
    }

    /// A run of years is not a run of disks, however tidily it varies.
    #[test]
    fn a_varying_year_is_not_an_index() {
        let g = group(&paths(&["Zork (1988)(Infocom).adf", "Zork (1989)(Infocom).adf"]));
        assert!(g.is_empty(), "1988/1989 reach neither 1 nor MAX_INDEX");
    }

    /// The index must reach 1. Stated as a test because it is the clause doing
    /// the work above, and because its cost — a set whose first disk is missing
    /// is no set — should fail loudly here if anyone relaxes it.
    #[test]
    fn a_set_without_its_first_disk_is_not_recognised() {
        assert!(group(&paths(&["floppy2.ima", "floppy3.ima", "floppy4.ima"])).is_empty());
        assert!(!group(&paths(&["floppy1.ima", "floppy2.ima"])).is_empty());
    }

    /// A lone volume is not a set, and neither is a repeated index.
    #[test]
    fn a_set_needs_two_distinct_disks() {
        assert!(group(&paths(&["floppy1.ima"])).is_empty());
        assert!(group(&paths(&["floppy1.ima", "floppy01.ima"])).is_empty(), "1 twice is not 1..2");
    }

    /// Roman numerals are text; three Zork floppies are three games.
    #[test]
    fn roman_numerals_are_not_indices() {
        let g = group(&paths(&[
            "Zork I - The Great Underground Empire.adf",
            "Zork II - The Wizard of Frobozz.adf",
            "Zork III - The Dungeon Master.adf",
        ]));
        assert!(g.is_empty());
    }

    /// Two digit runs that each make a plausible set leave the file in neither:
    /// guessing which is the disk number is the failure mode, so it is refused.
    #[test]
    fn an_ambiguous_stem_forms_no_set() {
        let g = group(&paths(&["v1 part1.img", "v1 part2.img", "v2 part1.img"]));
        assert!(g.is_empty(), "{:?}", g.iter().map(|x| names_of(x)).collect::<Vec<_>>());
    }

    /// Extensions are matched case-insensitively, and stems are not.
    #[test]
    fn extensions_are_case_insensitive() {
        let g = group(&paths(&["DISK1.IMG", "DISK2.IMG"]));
        assert_eq!(g.len(), 1);
    }

    #[test]
    fn members_of_a_non_image_is_none() {
        assert!(members(Path::new("/stories/zork1.z5")).is_none());
    }

    /// **A single raw disk is not a set of one** (SQ-0868), even sitting in a
    /// directory full of `.dsk` volumes that are.
    ///
    /// `Planetfall r29 (clean copy from retail disk).dsk` is the corpus's first
    /// `.dsk` that is a whole game on one disk, and its stem carries a digit run
    /// (`r29`) — so the two things that could go wrong are that it forms a
    /// degenerate set of its own, or that it is dragged into Shogun's or Zork
    /// Zero's. Neither can: a set needs two files whose stems differ at ONE digit
    /// run, and no other file in the corpus shares this stem at all.
    ///
    /// This matters more than a lone-volume test usually would, because the
    /// consequence is not cosmetic. `MountedDisk::mount_set` consults a set's
    /// companions, and `dedupe_within_sets` folds rows sharing an IFID *within a
    /// set* — so a false set here is how a game silently disappears from the
    /// browser.
    #[test]
    fn a_lone_raw_disk_forms_no_set_among_the_sets_beside_it() {
        const RAW: &str = "Planetfall r29 (clean copy from retail disk).dsk";
        let files = paths(&[
            RAW,
            "shogun_s1.dsk",
            "shogun_s2.dsk",
            "zork_zero_1.dsk",
            "zork_zero_2.dsk",
        ]);
        let sets = group(&files);
        assert_eq!(sets.len(), 2, "the two presses are sets and the raw disk is not");
        for set in &sets {
            assert!(
                !names_of(set).iter().any(|n| n == RAW),
                "the raw disk was folded into {:?}",
                names_of(set)
            );
        }
        // …and on its own it is still nothing, digit run notwithstanding.
        assert!(group(&paths(&[RAW])).is_empty());
    }
}
