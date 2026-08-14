//! End-to-end: point the compiled `zvm-cli` at a disk image and let it mount
//! the story off it (SQ-0834).
//!
//! Two kinds of disk here. The **real** ones come out of the gitignored
//! `stories/` tree — the only proof that the mount reads genuine media, and
//! vacuously skipped when a fixture is absent. The **synthetic** ones are built
//! here, because every Amiga floppy in the corpus carries exactly one story and
//! the menu only appears when a disk carries several.
//!
//! Since SQ-0833/SQ-0835 the menu has real disks to stand on: a DOS compilation
//! carrying six games and an Atari ST one carrying four, where the ST's four are
//! all called `STORY.DAT` and only the directory tells them apart.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

// ── running the binary ────────────────────────────────────────────────────────

/// Run `zvm-cli <image> [args]` with `stdin_script` piped in — so stdin is
/// never a terminal, which is the non-interactive path under test.
fn run(image: &std::path::Path, extra_args: &[&str], stdin_script: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_zvm-cli"))
        .arg(image)
        .args(extra_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zvm-cli spawns");
    child.stdin.take().unwrap().write_all(stdin_script.as_bytes()).unwrap();
    child.wait_with_output().expect("zvm-cli runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

// ── synthetic media ───────────────────────────────────────────────────────────

/// AmigaDOS block size.
const BSIZE: usize = 512;
/// Blocks on a real 880 KB DD floppy.
const DD_BLOCKS: usize = 1760;

/// A minimal AmigaDOS (FFS) disk image builder. Only what a file needs: a
/// bootblock, and a header block per file with its data blocks listed in the
/// reverse-order table at `BSIZE-204`. Files here are small enough never to
/// need an extension block.
struct Floppy {
    image: Vec<u8>,
    next: usize,
}

impl Floppy {
    fn new() -> Floppy {
        let mut image = vec![0u8; DD_BLOCKS * BSIZE];
        image[0..3].copy_from_slice(b"DOS");
        image[3] = 1; // FFS: data blocks are raw 512-byte payload
        Floppy { image, next: 881 } // files start after the root block, as on a real disk
    }

    fn put32(&mut self, block: usize, off: usize, v: u32) {
        let at = block * BSIZE + off;
        self.image[at..at + 4].copy_from_slice(&v.to_be_bytes());
    }

    fn add_file(&mut self, name: &str, data: &[u8]) -> &mut Floppy {
        let header = self.next;
        self.next += 1;
        self.put32(header, 0, 2); // T_HEADER
        self.put32(header, 4, header as u32); // a header names its own block
        self.put32(header, BSIZE - 4, 0xFFFF_FFFD); // ST_FILE
        self.put32(header, BSIZE - 188, data.len() as u32);
        let at = header * BSIZE + BSIZE - 80;
        self.image[at] = name.len() as u8;
        self.image[at + 1..at + 1 + name.len()].copy_from_slice(name.as_bytes());

        for (i, chunk) in data.chunks(BSIZE).enumerate() {
            let db = self.next;
            self.next += 1;
            let at = db * BSIZE;
            self.image[at..at + chunk.len()].copy_from_slice(chunk);
            self.put32(header, BSIZE - 204 - 4 * i, db as u32);
        }
        self.put32(header, 8, data.len().div_ceil(BSIZE) as u32); // high_seq
        self
    }

    /// Write the image beside the test binary and return its path.
    fn write(&self, name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("zvm-cli-{}-{name}", std::process::id()));
        std::fs::write(&path, &self.image).expect("temp image written");
        path
    }
}

/// A story that quits the moment it starts: enough header for `looks_like_story`
/// to recognise it and for `zvm` to boot it, and `@quit` at the initial PC so
/// the run ends without waiting for input.
fn quitting_story(version: u8, release: u16, serial: &str) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    b[0x00] = version;
    b[0x02..0x04].copy_from_slice(&release.to_be_bytes());
    let mut word = |o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_be_bytes());
    word(0x04, 0x0400); // high memory
    word(0x06, 0x0040); // initial PC
    word(0x08, 0x0300); // dictionary
    word(0x0a, 0x0100); // objects
    word(0x0c, 0x0200); // globals
    word(0x0e, 0x0280); // static memory base
    word(0x18, 0x0060); // abbreviations
    word(0x1a, (0x400 / if version == 3 { 2 } else { 4 }) as u16); // declared length
    b[0x12..0x18].copy_from_slice(serial.as_bytes());
    b[0x40] = 0xBA; // 0OP:0x0A quit
    b
}

/// A story that prints header byte `$1E` — the interpreter number the host told
/// it it is running on — between brackets, then quits (SQ-0839).
///
/// Hand-assembled at the initial PC, which for v1–v5 is a plain byte address
/// rather than a routine: `print_char '['` (VAR:0x05), `loadb 0 $1E -> sp`
/// (2OP:0x10, long form, both operands small constants), `print_num sp`
/// (VAR:0x06, operand on the stack), `print_char ']'`, `quit`. The brackets are
/// what make the assertion safe — a bare digit would match the score line.
fn interpreter_reporting_story() -> Vec<u8> {
    let mut b = quitting_story(3, 88, "840726");
    b[0x40..0x4E].copy_from_slice(&[
        0xE5, 0x7F, 0x5B, // print_char '['
        0x10, 0x00, 0x1E, 0x00, // loadb 0 $1E -> sp
        0xE6, 0xBF, 0x00, // print_num sp
        0xE5, 0x7F, 0x5D, // print_char ']'
        0xBA, // quit
    ]);
    b
}

/// Two stories on one disk, named and released differently so the menu has
/// something to tell apart.
fn two_story_floppy(name: &str) -> PathBuf {
    let mut f = Floppy::new();
    f.add_file("Zork1.Data", &quitting_story(3, 88, "840726"));
    f.add_file("Zork2.Data", &quitting_story(5, 48, "840904"));
    f.write(name)
}

// ── real media ────────────────────────────────────────────────────────────────

/// The gitignored `stories/` tree, two levels up from this crate.
fn story_path(name: &str) -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join("stories").join(name);
    if p.exists() {
        return Some(p);
    }
    eprintln!("SKIP: gitignored disk image missing at {}", p.display());
    None
}

/// The original Amiga *Zork I* floppy carries one story (`Zork1.Data`, v3 r88
/// s840726) — so it opens straight into the game with nothing asked, and the
/// game really is running: "West of House" is Zork's opening room.
#[test]
fn a_single_story_floppy_opens_without_asking() {
    let Some(image) = story_path("Zork I - The Great Underground Empire.adf") else { return };
    let out = run(&image, &[], "quit\ny\n");
    let text = stdout_of(&out);
    assert!(text.contains("West of House"), "the story mounted off the floppy ran:\n{text}");
    assert!(!text.contains("Which one?"), "one story is not a choice:\n{text}");
    assert!(!stderr_of(&out).contains("--story"), "nothing to pick:\n{}", stderr_of(&out));
}

// ── every format, not just the Amiga's (SQ-0840) ──────────────────────────────

/// A bare, structurally valid HFS volume with an empty catalog — unmistakably a
/// Macintosh disk, and nothing like an AmigaDOS one. Signature `BD` a kilobyte
/// in, then a Master Directory Block whose geometry describes the volume it sits
/// in. No fixture needed, so this never skips.
fn macintosh_volume() -> Vec<u8> {
    let mut v = vec![0u8; 1600 * BSIZE];
    let mdb = 2 * BSIZE;
    v[mdb..mdb + 2].copy_from_slice(&0x4244u16.to_be_bytes()); // drSigWord
    v[mdb + 18..mdb + 20].copy_from_slice(&1596u16.to_be_bytes()); // drNmAlBlks
    v[mdb + 20..mdb + 24].copy_from_slice(&512u32.to_be_bytes()); // drAlBlkSiz
    v[mdb + 28..mdb + 30].copy_from_slice(&4u16.to_be_bytes()); // drAlBlSt
    v
}

/// **The bug this quest was filed for.** `zvm-cli` refused a Macintosh disk
/// outright — `blorb` had read that filesystem since SQ-0837, but this
/// front-end's detector was pinned to `Adf`, so the raw image went to the VM as
/// if it were a story file.
///
/// A Mac volume must now be *mounted* like any other disk, and the message must
/// be about what is on the disk. This one has nothing on it, so that message is
/// the boot-disk one.
///
/// FALSIFICATION: restore `looks_like_image` to `== Some(DiskImage::Adf)` and
/// this fails with the symptom as reported —
/// `Error: Z-machine version 0 is not supported.` — byte 0 of an unmounted
/// volume, read as a story header.
#[test]
fn a_macintosh_volume_is_mounted_like_any_other_disk() {
    let image = write_temp("mac-empty.image", &macintosh_volume());
    let out = run(&image, &[], "");
    assert!(!out.status.success(), "an empty volume holds no game");
    let err = stderr_of(&out);
    assert!(err.contains("no story file on this disk image"), "mounted, and said so:\n{err}");
    assert!(!err.contains("is not supported"), "never handed to the VM as a story:\n{err}");
    let _ = std::fs::remove_file(&image);
}

/// The real Macintosh release disk: *Zork Zero* release 296 / serial 881019, an
/// HFS volume inside a DiskCopy 4.2 wrapper. It mounts, and is then declined at
/// load because `zvm-cli` renders no v6 graphics — which is this front-end's own
/// limitation and the same answer the five graphical Amiga floppies get. **The
/// user must see the v6 refusal, not a disk error**; that is what says the mount
/// worked.
///
/// FALSIFICATION: with `looks_like_image` narrowed back to `Adf`, this reads
/// `Error: Z-machine version 14 is not supported.` — 14 being the Pascal length
/// of the DiskCopy header's disk name, `Zork Zero Disk`.
#[test]
fn the_real_macintosh_release_disk_mounts_and_is_declined_as_v6() {
    let Some(image) = story_path("Zork Zero Disk.image") else { return };
    let out = run(&image, &[], "");
    let err = stderr_of(&out);
    assert!(err.contains("v6 graphical games are not supported"), "the v6 refusal:\n{err}");
    assert!(!err.contains("version"), "not a version-number complaint about the image:\n{err}");
    assert!(!err.contains("disk image"), "not a disk error — the disk mounted fine:\n{err}");
}

/// The same disk on the other medium, for contrast: the Amiga *Zork Zero* is
/// also v6 and gets the identical refusal. Two formats, one behaviour — which is
/// the whole of the user's rule.
#[test]
fn an_amiga_v6_floppy_is_declined_exactly_as_the_macintosh_one_is() {
    let Some(amiga) = story_path("Zork Zero - The Revenge of Megaboz.adf") else { return };
    let Some(mac) = story_path("Zork Zero Disk.image") else { return };
    assert_eq!(
        stderr_of(&run(&amiga, &[], "")),
        stderr_of(&run(&mac, &[], "")),
        "the medium changes nothing about how the CLI declines a v6 game"
    );
}

// ── the menu ──────────────────────────────────────────────────────────────────

/// Nobody at the keyboard and more than one story: list what is there, name the
/// flag, and stop. A prompt here would hang every script and every test.
#[test]
fn several_stories_and_no_terminal_lists_them_and_refuses_to_block() {
    let image = two_story_floppy("list.adf");
    let out = run(&image, &[], "");
    assert!(!out.status.success(), "no story was chosen, so this cannot succeed");
    let err = stderr_of(&out);
    assert!(err.contains("holds 2 stories"), "says what it found:\n{err}");
    assert!(err.contains("--story <n|name>"), "names the flag to pass:\n{err}");
    assert!(err.contains("1) Zork1.Data  (v3 r88 s840726)"), "labels the first:\n{err}");
    assert!(err.contains("2) Zork2.Data  (v5 r48 s840904)"), "labels the second:\n{err}");
    let _ = std::fs::remove_file(&image);
}

/// `--story <n>` picks by menu number, and the chosen story is the one that
/// boots — the run reaches `@quit` and exits cleanly.
#[test]
fn story_number_picks_one_off_the_disk() {
    let image = two_story_floppy("number.adf");
    let out = run(&image, &["--story", "2"], "");
    let text = stdout_of(&out);
    assert!(out.status.success(), "the chosen story ran: {}", stderr_of(&out));
    assert!(text.contains("Opening 2) Zork2.Data  (v5 r48 s840904)"), "says which:\n{text}");
    let _ = std::fs::remove_file(&image);
}

/// `--story <name>` picks by name, case-insensitively and on a partial. On this
/// medium the names differ; on an Atari ST compilation every story is called
/// `STORY.DAT`, which is why the number always works too.
#[test]
fn story_name_picks_one_off_the_disk() {
    let image = two_story_floppy("name.adf");
    let out = run(&image, &["--story", "zork1"], "");
    let text = stdout_of(&out);
    assert!(out.status.success(), "the chosen story ran: {}", stderr_of(&out));
    assert!(text.contains("Opening 1) Zork1.Data  (v3 r88 s840726)"), "says which:\n{text}");

    let out = run(&image, &["--story", "zork9"], "");
    assert!(!out.status.success(), "a name that matches nothing cannot open a story");
    assert!(stderr_of(&out).contains("is named 'zork9'"), "{}", stderr_of(&out));
    let _ = std::fs::remove_file(&image);
}

// ── real compilations, on both FAT12 machines (SQ-0833, SQ-0835) ──────────────

/// **The menu the synthetic disks above were standing in for.** Until now every
/// real floppy in the corpus carried exactly one story, so the choose-a-story
/// path was only ever exercised against a disk this file built. *The Lost
/// Treasures of Infocom* I, floppy2 carries six, and they are six different
/// games with six different headers.
///
/// `HITCHHIK.DAT` here is the Solid Gold **v5 r31 s871119** — a different
/// release AND a different Z-machine version from the standalone DOS disk's v3
/// r58 s851002 and from the Atari ST's v3 r56 s841221. Which is why the menu
/// shows the header beside the name: on this medium the names differ, and on
/// the next one they do not.
#[test]
fn a_real_dos_compilation_lists_six_games_and_story_opens_one() {
    let Some(image) = story_path("floppy2.ima") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    assert!(err.contains("holds 6 stories"), "the whole disk is offered:\n{err}");
    assert!(err.contains("1) HITCHHIK.DAT  (v5 r31 s871119)"), "the Solid Gold release:\n{err}");
    assert!(err.contains("4) BALLYHOO.DAT  (v3 r97 s851218)"), "…and the rest of them:\n{err}");

    let out = run(&image, &["--story", "hitchhik"], "quit\ny\n");
    let text = stdout_of(&out);
    assert!(text.contains("Opening 1) HITCHHIK.DAT"), "says which it opened:\n{text}");
    assert!(
        text.contains("Release 31 / Serial number 871119"),
        "and the GAME says which release it is:\n{text}"
    );
}

/// The Atari ST, where the filename gives nothing away: four games, four
/// folders, and four files called `STORY.DAT`. The **directory** is the only
/// thing that tells them apart, so it is what the menu shows — which is the
/// whole of SQ-0835's naming finding, visible at the prompt.
///
/// FALSIFICATION: have `blorb::fat12` report `Fat12Entry::name` instead of
/// `Fat12Entry::path` and this menu becomes four identical `STORY.DAT` lines,
/// distinguishable only by their headers and unpickable by `--story <name>`.
#[test]
fn a_real_atari_st_compilation_names_its_games_by_directory() {
    let Some(image) = story_path("Infocom Compilation 9 (19xx)(-).st") else { return };
    let err = stderr_of(&run(&image, &[], ""));
    assert!(err.contains("holds 4 stories"), "four games, and no saved game among them:\n{err}");
    for line in [
        "1) HITCHHIK/STORY.DAT  (v3 r56 s841221)",
        "2) BUREAUCR.ACY/STORY.DAT  (v4 r86 s870212)",
        "3) CUTHROAT/STORY.DAT  (v3 r23 s840809)",
        "4) LEATHER.GOD/STORY.DAT  (v3 r59 s860730)",
    ] {
        assert!(err.contains(line), "expected {line:?} in:\n{err}");
    }
    assert!(!err.contains(".SAV"), "the disk's 1996 saved games are not games:\n{err}");

    // …and the directory name is enough to pick one, which a bare `STORY.DAT`
    // never could be.
    let out = run(&image, &["--story", "cuthroat"], "quit\ny\n");
    let text = stdout_of(&out);
    assert!(text.contains("Opening 3) CUTHROAT/STORY.DAT"), "says which it opened:\n{text}");
    assert!(text.contains("Hardscrabble Island"), "…and Cutthroats really ran:\n{text}");
}

// ── the medium picks the machine (SQ-0839) ────────────────────────────────────

/// Write bytes to a temp file and return the path.
fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("zvm-cli-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).expect("temp file written");
    path
}

/// The rule, and its override, on one disk: a story mounted off an Amiga floppy
/// is told it is running on an Amiga — interpreter 4, ZMSD §11.1.3 — and `-I`
/// still outranks the medium, because the number you name is the machine you
/// asked for.
///
/// FALSIFICATION: drop the `.or_else(|| medium.and_then(…))` in `main.rs` and
/// the first assertion fails with the reported symptom — `[1]`, the IBM PC
/// default, on a disk that is not one.
#[test]
fn a_story_off_an_amiga_floppy_is_told_it_is_an_amiga() {
    let mut f = Floppy::new();
    f.add_file("Story.data", &interpreter_reporting_story());
    let image = f.write("machine.adf");

    let text = stdout_of(&run(&image, &[], ""));
    assert!(text.contains("[4]"), "the medium defaults header $1E to the Amiga's 4:\n{text}");

    // The contract: an explicit number beats the medium, in both directions.
    let text = stdout_of(&run(&image, &["-I", "6"], ""));
    assert!(text.contains("[6]"), "-I 6 outranks the Amiga floppy it was opened from:\n{text}");
    let text = stdout_of(&run(&image, &["--interpreter", "3"], ""));
    assert!(text.contains("[3]"), "--interpreter is the same flag, same rank:\n{text}");

    let _ = std::fs::remove_file(&image);
}

/// …and the blast radius is exactly zero for everything that is not a medium.
/// An ordinary story file keeps zvm's own rule (Frotz's: 1 below Version 6), so
/// this feature cannot move a single story anybody already plays.
#[test]
fn an_ordinary_story_file_keeps_the_default_interpreter() {
    let story = write_temp("plain.z3", &interpreter_reporting_story());
    let text = stdout_of(&run(&story, &[], ""));
    assert!(text.contains("[1]"), "no medium, no opinion — zvm's own rule stands:\n{text}");
    let text = stdout_of(&run(&story, &["-I", "4"], ""));
    assert!(text.contains("[4]"), "and asking for a machine still works without one:\n{text}");
    let _ = std::fs::remove_file(&story);
}

/// The real-media proof, and the one that matters: a genuine Infocom release
/// floppy, driven as a real game, answering VERSION in its own words.
///
/// *Zork: The Undiscovered Underground* (release 16 / serial 970828) is
/// Inform-compiled, and the Inform library's banner prints header `$1E` outright
/// — so the game itself says which machine it was told about. The app-side
/// suite (`real_media_releases.rs`) pins the same line off the same floppy; this
/// is the CLI reaching the identical conclusion, which is the whole point of
/// SQ-0839.
///
/// FALSIFICATION: with the `main.rs` default reverted this reads
/// `Interpreter 1 Version A`, which is the bug as reported — an Amiga floppy
/// mounted and then run as an IBM PC.
#[test]
fn a_real_release_floppy_tells_the_game_which_machine_it_is() {
    let Some(image) = story_path("Zork - The Undiscovered Underground.adf") else { return };

    let text = stdout_of(&run(&image, &["--no-more"], "version\nquit\ny\n"));
    assert!(
        text.contains("Release 16 / Serial number 970828"),
        "this is the build the floppy carries:\n{text}"
    );
    assert!(
        text.contains("Interpreter 4 "),
        "the game names the Amiga it was told it is running on:\n{text}"
    );

    // And the override, on the same real media.
    let text = stdout_of(&run(&image, &["--no-more", "-I", "6"], "version\nquit\ny\n"));
    assert!(text.contains("Interpreter 6 "), "-I outranks the floppy:\n{text}");
}

/// A disk with no story on it — an AmigaDOS boot disk, say — says so, rather
/// than failing later as if the image were a corrupt story file.
#[test]
fn a_disk_with_no_story_says_what_it_mounted() {
    let mut f = Floppy::new();
    f.add_file("Startup-Sequence", b"echo hello\n");
    let image = f.write("boot.adf");
    let out = run(&image, &[], "");
    assert!(!out.status.success());
    let err = stderr_of(&out);
    assert!(err.contains("no story file on this disk image"), "{err}");
    assert!(err.contains("1 file mounted"), "says what it did find:\n{err}");
    let _ = std::fs::remove_file(&image);
}
