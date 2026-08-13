//! End-to-end: point the compiled `zvm-cli` at a disk image and let it mount
//! the story off it (SQ-0834).
//!
//! Two kinds of disk here. The **real** one is an original Amiga release floppy
//! out of the gitignored `stories/` tree — the only proof that the mount reads
//! genuine media, and vacuously skipped when the fixture is absent. The
//! **synthetic** ones are built here, because every Amiga floppy in the corpus
//! carries exactly one story and the menu only appears when a disk carries
//! several (the DOS and ST compilations do, and land under SQ-0833).

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
