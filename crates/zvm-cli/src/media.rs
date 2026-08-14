//! Opening an original release disk image.
//!
//! `blorb` mounts release floppies for the TUI, and `zvm-cli` already links
//! `blorb` — so pointing this front-end at one costs no new dependency and no
//! new parsing, only the wiring in this file (SQ-0834).
//!
//! **Every format `blorb` reads, this front-end opens** (SQ-0840). Nothing here
//! names a filesystem: `blorb::medium` is asked whether the bytes are a disk and
//! what is on it, so an Amiga floppy and a Macintosh volume arrive by the same
//! road and the next format arrives without this file changing. That is the
//! user's rule — *"please keep the functionality consistent for all disk image
//! formats"* — made structural rather than remembered.
//!
//! The one thing a disk needs that a bare story file does not is a **choice**.
//! An original single-game floppy carries one story and opens straight away,
//! but a compilation disk carries several, so a mounted image yields a list and
//! somebody has to pick from it: the player at a prompt, or `--story` when
//! nobody is watching.

use blorb::medium::{DiskImage, MountedDisk};

/// One story found on a mounted disk image.
pub struct Candidate {
    /// The best name the medium gives this story: the stored filename, prefixed
    /// by its directory on a format that has them (`HITCHHIK/STORY.DAT`).
    /// AmigaDOS release floppies name every story `Story.data` and Atari ST
    /// compilations name every one `STORY.DAT`, so on a flat disk this rarely
    /// distinguishes anything — which is why the menu also shows
    /// [`Candidate::header`].
    pub name: String,
    /// The story bytes, read off the image.
    pub bytes: Vec<u8>,
}

impl Candidate {
    /// The Z-machine version, release and serial, e.g. `v3 r88 s840726`.
    ///
    /// `None` when the bytes are not Z-code (a Blorb or Glulx image on a disk
    /// would be), because then there is no header to read. This is what
    /// actually tells two candidates apart: the corpus holds three different
    /// releases of *Hitchhiker's* alone (v3 r56 s841221, v3 r58 s851002,
    /// v5 r31 s871119).
    pub fn header(&self) -> Option<String> {
        let b = &self.bytes;
        if b.len() < 0x18 || !(3..=8).contains(&b[0]) {
            return None;
        }
        let serial: String = b[0x12..0x18].iter().map(|c| char::from(*c)).collect();
        if !serial.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
            return None;
        }
        let release = u16::from_be_bytes([b[0x02], b[0x03]]);
        Some(format!("v{} r{release} s{serial}", b[0]))
    }

    /// How this candidate reads in the menu: its name, then its header when
    /// there is one.
    pub fn label(&self) -> String {
        match self.header() {
            Some(h) => format!("{}  ({h})", self.name),
            None => self.name.clone(),
        }
    }
}

/// Are these bytes a disk image this front-end can mount?
///
/// Content, not extension — exactly as the TUI asks the question, of exactly the
/// same recogniser, so an image with any name is recognised and a mis-named
/// story file is not.
///
/// It used to be **narrower** than [`blorb::medium::DiskImage::detect`], pinned
/// to `Adf` alone with a comment explaining that this front-end must not claim
/// an image it cannot open — an honest guard around a real hole, since
/// `story_candidates` had no HFS arm and a Macintosh disk would have detected
/// and then failed. The hole is gone: detect and mount now walk one table
/// (SQ-0840), so whatever `blorb` recognises, `blorb` opens, and the guard has
/// nothing left to guard.
pub fn looks_like_image(raw: &[u8]) -> bool {
    DiskImage::detect(raw).is_some()
}

/// Mount `raw` and return every story on it, in disk order.
///
/// Identified by content: a release disk's filenames prove nothing — AmigaDOS
/// has no extensions and every Atari ST story is called `STORY.DAT` — so
/// `blorb` decides by the bytes, strictly enough to reject the saved games the
/// original *Zork Zero* floppy carries.
pub fn story_candidates(raw: Vec<u8>) -> Result<Vec<Candidate>, String> {
    let disk =
        MountedDisk::mount(raw).map_err(|e| format!("Error: cannot mount the disk image: {e}"))?;
    let mounted = disk.file_count();
    let found: Vec<Candidate> = disk
        .stories()
        .into_iter()
        .map(|s| Candidate { name: s.name, bytes: s.bytes })
        .collect();
    if found.is_empty() {
        let files = if mounted == 1 { "file" } else { "files" };
        // Formats that keep a volume name say it; the ones that do not read the
        // same as they always did.
        let named = disk.volume_name().map(|n| format!(" on {n}")).unwrap_or_default();
        return Err(format!(
            "Error: no story file on this disk image \
             ({mounted} {files} mounted{named}; is this the boot disk?)"
        ));
    }
    Ok(found)
}

/// The numbered list a player picks from.
pub fn menu(cands: &[Candidate]) -> String {
    let mut s = String::new();
    for (i, c) in cands.iter().enumerate() {
        s.push_str(&format!("  {}) {}\n", i + 1, c.label()));
    }
    s
}

/// Resolve what `--story` asked for: a 1-based number, or a name to match
/// (case-insensitive, and a substring is enough as long as it picks out one
/// story).
pub fn find(cands: &[Candidate], want: &str) -> Result<usize, String> {
    let want = want.trim();
    if let Ok(n) = want.parse::<usize>() {
        if (1..=cands.len()).contains(&n) {
            return Ok(n - 1);
        }
        let last = cands.len();
        return Err(format!("no story {n} on this disk — pick 1 to {last}:\n{}", menu(cands)));
    }
    let lower = want.to_ascii_lowercase();
    let hits: Vec<usize> = (0..cands.len())
        .filter(|&i| cands[i].name.to_ascii_lowercase().contains(&lower))
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(format!("no story on this disk is named '{want}':\n{}", menu(cands))),
        _ => Err(format!("'{want}' matches more than one story on this disk:\n{}", menu(cands))),
    }
}

/// Pick a story off a mounted image.
///
/// One candidate opens without asking, whatever else is going on — the common
/// single-game floppy never sees a menu. `--story` decides it outright.
/// Otherwise the choice needs a person: with a terminal on stdin the menu is
/// printed and `read_line` answers it; without one this **refuses to block**
/// and says which flag to pass, because a prompt nobody can answer is a hang
/// (and would make this untestable and unscriptable).
///
/// `read_line` returns `None` at end of input. `announce` receives the menu and
/// the prompt, so the caller decides where they are printed and a test can keep
/// them.
pub fn choose(
    cands: &[Candidate],
    want: Option<&str>,
    stdin_is_tty: bool,
    mut announce: impl FnMut(&str),
    mut read_line: impl FnMut() -> Option<String>,
) -> Result<usize, String> {
    if let Some(w) = want {
        return find(cands, w);
    }
    if cands.len() == 1 {
        return Ok(0);
    }
    if !stdin_is_tty {
        return Err(format!(
            "Error: this disk image holds {} stories; pass --story <n|name> to pick one:\n{}",
            cands.len(),
            menu(cands)
        ));
    }
    announce(&format!("This disk holds {} stories:\n{}", cands.len(), menu(cands)));
    loop {
        announce(&format!("Which one? [1-{}] ", cands.len()));
        let Some(line) = read_line() else {
            return Err("Error: no story chosen.".to_string());
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match find(cands, line) {
            Ok(i) => return Ok(i),
            Err(e) => announce(&format!("{e}\n")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare, structurally valid HFS volume with an empty catalog: the
    /// cheapest thing that is unmistakably a Macintosh disk and not an
    /// AmigaDOS one. Signature `BD` a kilobyte in, then a Master Directory
    /// Block whose geometry describes the volume it sits in.
    fn macintosh_volume() -> Vec<u8> {
        let mut v = vec![0u8; 1600 * 512];
        let mdb = 2 * 512;
        v[mdb..mdb + 2].copy_from_slice(&0x4244u16.to_be_bytes()); // drSigWord
        v[mdb + 18..mdb + 20].copy_from_slice(&1596u16.to_be_bytes()); // drNmAlBlks
        v[mdb + 20..mdb + 24].copy_from_slice(&512u32.to_be_bytes()); // drAlBlkSiz
        v[mdb + 28..mdb + 30].copy_from_slice(&4u16.to_be_bytes()); // drAlBlSt
        v
    }

    /// An AmigaDOS floppy, boot block and all.
    fn amiga_floppy() -> Vec<u8> {
        let mut v = vec![0u8; 1760 * 512];
        v[0..3].copy_from_slice(b"DOS");
        v
    }

    /// **The rule, as a test**: whatever `blorb` recognises, this front-end
    /// claims — for every format, with no exceptions carved out here.
    ///
    /// FALSIFICATION: narrow `looks_like_image` back to `== Some(DiskImage::Adf)`
    /// and this fails on the Macintosh volume, which is exactly the reported
    /// bug — `zvm-cli` opened an Amiga floppy and refused a Mac disk that
    /// `blorb` had been able to read for a month.
    #[test]
    fn this_front_end_claims_every_disk_blorb_can_open() {
        for raw in [amiga_floppy(), macintosh_volume()] {
            let detected = DiskImage::detect(&raw).expect("a disk image");
            assert!(
                looks_like_image(&raw),
                "blorb detects {detected:?} but this front-end will not claim it"
            );
            // …and claiming it means being able to open it, not merely to name
            // it: a detector that claims a disk and then fails is worse than one
            // that declines it (SQ-0840).
            let mounted = MountedDisk::mount(raw).expect("what we claim, we can mount");
            assert_eq!(mounted.format(), detected);
        }
    }

    /// The other half: an ordinary story file is not a disk, and is not claimed.
    #[test]
    fn a_plain_story_file_is_not_claimed_as_a_disk() {
        assert!(!looks_like_image(&story(3, 88, "840726")));
        assert!(!looks_like_image(b"FORM\x00\x00\x00\x04IFRS"));
        assert!(!looks_like_image(&[]));
    }

    /// A story buffer whose header carries a given version, release and serial.
    /// Only the header is read here, so the rest can stay zero.
    fn story(v: u8, release: u16, serial: &str) -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[0x00] = v;
        b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
        b[0x12..0x18].copy_from_slice(serial.as_bytes());
        b
    }

    fn cand(name: &str, v: u8, release: u16, serial: &str) -> Candidate {
        Candidate { name: name.to_string(), bytes: story(v, release, serial) }
    }

    /// The whole point of showing the header: an Amiga floppy calls every story
    /// `Story.data`, and an ST compilation calls every one `STORY.DAT`, so the
    /// name alone makes a menu of identical lines.
    #[test]
    fn a_candidate_is_labelled_by_name_and_header() {
        let c = cand("Story.data", 3, 88, "840726");
        assert_eq!(c.header().as_deref(), Some("v3 r88 s840726"));
        assert_eq!(c.label(), "Story.data  (v3 r88 s840726)");
    }

    /// Nothing to read a header out of (a Blorb, say) still gets a usable line.
    #[test]
    fn a_candidate_without_a_readable_header_is_labelled_by_name_alone() {
        let c = Candidate { name: "game.blb".into(), bytes: b"FORM....IFRS".to_vec() };
        assert_eq!(c.header(), None);
        assert_eq!(c.label(), "game.blb");
    }

    #[test]
    fn one_story_opens_without_asking() {
        let cands = vec![cand("Story.data", 3, 88, "840726")];
        let mut asked = false;
        let i = choose(&cands, None, true, |_| asked = true, || panic!("must not read stdin"));
        assert_eq!(i, Ok(0));
        assert!(!asked, "a single-story disk asks nothing");
    }

    #[test]
    fn several_stories_are_listed_and_the_answer_selects_one() {
        let cands = vec![
            cand("Story.data", 3, 56, "841221"),
            cand("Story.data", 5, 31, "871119"),
            cand("Hints.data", 3, 22, "870918"),
        ];
        let mut said = String::new();
        let mut lines = ["2\n".to_string()].into_iter();
        let i = choose(&cands, None, true, |s| said.push_str(s), || lines.next());
        assert_eq!(i, Ok(1));
        assert!(said.contains("1) Story.data  (v3 r56 s841221)"), "menu shown:\n{said}");
        assert!(said.contains("2) Story.data  (v5 r31 s871119)"), "menu shown:\n{said}");
        assert!(said.contains("Which one? [1-3]"), "prompt shown:\n{said}");
    }

    /// A fat-fingered answer re-asks rather than giving up or opening the wrong
    /// game.
    #[test]
    fn a_bad_answer_is_asked_again() {
        let cands = vec![cand("Story.data", 3, 56, "841221"), cand("Other.data", 5, 31, "871119")];
        let mut said = String::new();
        let mut lines = ["9\n".to_string(), "other\n".to_string()].into_iter();
        let i = choose(&cands, None, true, |s| said.push_str(s), || lines.next());
        assert_eq!(i, Ok(1));
        assert!(said.contains("no story 9 on this disk"), "told what was wrong:\n{said}");
    }

    /// The requirement that makes this scriptable at all: no terminal means no
    /// prompt, ever.
    #[test]
    fn without_a_terminal_the_menu_refuses_to_block() {
        let cands = vec![cand("Story.data", 3, 56, "841221"), cand("Other.data", 5, 31, "871119")];
        let e = choose(&cands, None, false, |_| {}, || panic!("must not read stdin"))
            .expect_err("no terminal, so no prompt");
        assert!(e.contains("--story <n|name>"), "names the flag:\n{e}");
        assert!(e.contains("1) Story.data"), "lists what it found:\n{e}");
    }

    #[test]
    fn story_picks_by_number_or_by_name() {
        let cands =
            vec![cand("Story.data", 3, 56, "841221"), cand("HITCHHIK.DAT", 5, 31, "871119")];
        let never = || -> Option<String> { panic!("must not read stdin") };
        assert_eq!(choose(&cands, Some("1"), false, |_| {}, never), Ok(0));
        assert_eq!(choose(&cands, Some("hitchhik"), false, |_| {}, never), Ok(1));
        assert!(find(&cands, "3").is_err(), "out of range");
        assert!(find(&cands, "zork").is_err(), "no such name");
        assert!(find(&cands, ".dat").is_err_and(|e| e.contains("more than one")), "ambiguous");
    }
}
