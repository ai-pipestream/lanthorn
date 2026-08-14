//! SQ-0760: the release each **medium** actually carries, pinned.
//!
//! An Infocom Amiga release floppy is not the same story as the bare story file
//! sitting beside it in `stories/` — it is a *different build of the game*.
//! `Journey - The Quest Begins.adf` is release 30, serial 890322; `journey.z6`
//! is release 83, serial 890706, and the two do not behave alike: r83 narrates
//! through window 0, r30 through window 2. That difference was the whole of
//! SQ-0755's reopened defect, and it cost SQ-0747 five investigation passes,
//! three of them sweeping tens of thousands of configurations against a release
//! the user does not play.
//!
//! `InterpreterProfile::resolve` reads the MEDIUM, so "under the Amiga profile"
//! names a machine *and* — when the fixture is a disk image — a different build.
//! Until this file there was **no committed test that drove a floppy at all**:
//! the SQ-0755 corpus measured both media for Zork Zero, Shogun and Arthur and
//! found them to agree, but an agreement measured once and never pinned is
//! exactly how this slipped through the first time.
//!
//! So: one table, [`MEDIA`], naming every medium in `stories/` that has a
//! floppy counterpart together with the version, release and serial it must
//! load. Every case walks it, guards on that identity FIRST, and prefixes every
//! assertion message with [`ctx`] — the file, the release and the serial it
//! loaded — so a failure here can never again be attributed to the wrong build.
//!
//! `stories/` is gitignored (commercial media), so every case skips vacuously
//! per missing file, exactly like the other real-game smokes. The `ran > 0`
//! guards catch a LOCAL run whose filenames drifted; they are gated on
//! [`any_real_media_present`] so they cannot fire where the fixtures legitimately
//! cannot exist.

use std::path::PathBuf;
use std::sync::Mutex;

use app::engine::Engine;
use app::graphics::PictSource;
use app::hints::DiskImage;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global (an Amiga medium loads the
/// Amiga palette), so no two cases here may boot at once.
static PALETTE: Mutex<()> = Mutex::new(());

// ── The table ────────────────────────────────────────────────────────────────

/// One story image as it exists on one medium.
#[derive(Clone, Copy)]
struct Medium {
    /// The game, so a pair reads as a pair.
    title: &'static str,
    /// Filename under `stories/`.
    file: &'static str,
    /// The release disk image this medium is, if it is one rather than a bare
    /// story file — and which filesystem, because that is what says which
    /// machine's media it is.
    image: Option<DiskImage>,
    /// Z-machine version, header $00.
    version: u8,
    /// Release number, header $02.
    release: u16,
    /// Serial, header $12..$18.
    serial: &'static str,
}

/// Every medium in `stories/` worth pinning a build against, with the build each
/// one carries. **Measured**, 2026-08-10 and extended 2026-08-13 for the DOS and
/// Atari ST presses, by mounting each file through
/// `app::hints::load_mounted_story` and reading its header.
///
/// Read the pairs: four of the five titles that ship both ways ship *different
/// builds*, and the four v3/v5 pairs ship the same one. Then read the
/// Hitchhiker's trio at the bottom, which is the same lesson at three media and
/// two Z-machine versions.
const MEDIA: &[Medium] = &[
    // Journey — the pair that started this. Different builds, and they differ
    // in which window they narrate through (see `V6_FRAMES`).
    Medium { title: "Journey", file: "Journey - The Quest Begins.adf", image: Some(DiskImage::Adf), version: 6, release: 30, serial: "890322" },
    Medium { title: "Journey", file: "journey-r83-s890706.z6", image: None, version: 6, release: 83, serial: "890706" },
    // Zork Zero ships on THREE media here, and all three are different builds.
    // The Macintosh disk is the outlier by a mile: r296/881019 is October 1988,
    // where both others are 1989 (SQ-0837).
    Medium { title: "Zork Zero (Macintosh)", file: "Zork Zero Disk.image", image: Some(DiskImage::Hfs), version: 6, release: 296, serial: "881019" },
    // Zork Zero — different builds, and the floppy lays its story window out at
    // a different place and size.
    Medium { title: "Zork Zero", file: "Zork Zero - The Revenge of Megaboz.adf", image: Some(DiskImage::Adf), version: 6, release: 366, serial: "890323" },
    Medium { title: "Zork Zero", file: "zork0-r393-s890714.z6", image: None, version: 6, release: 393, serial: "890714" },
    // Shogun — different builds that (so far) lay out identically.
    Medium { title: "Shogun", file: "James Clavell's Shogun.adf", image: Some(DiskImage::Adf), version: 6, release: 295, serial: "890321" },
    Medium { title: "Shogun", file: "shogun-r322-s890706.z6", image: None, version: 6, release: 322, serial: "890706" },
    // Arthur — different builds that (so far) lay out identically.
    Medium { title: "Arthur", file: "Arthur - The Quest for Excalibur.adf", image: Some(DiskImage::Adf), version: 6, release: 54, serial: "890606" },
    Medium { title: "Arthur", file: "arthur-r74-s890714.z6", image: None, version: 6, release: 74, serial: "890714" },
    // Beyond Zork — the SAME build on both media.
    Medium { title: "Beyond Zork", file: "Beyond Zork - The Coconut of Quendor.adf", image: Some(DiskImage::Adf), version: 5, release: 57, serial: "871221" },
    Medium { title: "Beyond Zork", file: "beyondzork-r57-s871221.z5", image: None, version: 5, release: 57, serial: "871221" },
    // The Zork trilogy — the same build on both media.
    Medium { title: "Zork I", file: "Zork I - The Great Underground Empire.adf", image: Some(DiskImage::Adf), version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork I", file: "zork1-r88-s840726.z3", image: None, version: 3, release: 88, serial: "840726" },
    Medium { title: "Zork II", file: "Zork II - The Wizard of Frobozz.adf", image: Some(DiskImage::Adf), version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork II", file: "zork2-r48-s840904.z3", image: None, version: 3, release: 48, serial: "840904" },
    Medium { title: "Zork III", file: "Zork III - The Dungeon Master.adf", image: Some(DiskImage::Adf), version: 3, release: 17, serial: "840727" },
    Medium { title: "Zork III", file: "zork3-r17-s840727.z3", image: None, version: 3, release: 17, serial: "840727" },
    // Zork: The Undiscovered Underground ships on a floppy only.
    Medium { title: "ZTUU", file: "Zork - The Undiscovered Underground.adf", image: Some(DiskImage::Adf), version: 5, release: 16, serial: "970828" },
    // ── The PC and the Atari ST (SQ-0833, SQ-0835) ───────────────────────────
    //
    // Zork Zero's DOS press, on the disk its STORY is actually on: *The Lost
    // Treasures of Infocom* I, floppy5 — with its EGA art beside it, while its
    // CGA art sits on floppy4 and is unreachable from here. r393/890714, the
    // same build as the bare `zork0-r393-s890714.z6`, byte for byte.
    Medium { title: "Zork Zero (DOS)", file: "floppy5.ima", image: Some(DiskImage::Fat12Dos), version: 6, release: 393, serial: "890714" },
    // **Three media, three Hitchhiker's, two Z-machine versions.** This is the
    // project's "a disk image is a different release" rule at its most extreme,
    // and now it is pinned rather than asserted: the standalone DOS disk is v3
    // r58, the Lost Treasures collection ships the later Solid Gold v5 r31, and
    // the Atari ST press is v3 r56. A finding about "Hitchhiker's" that does not
    // name its medium describes none of them.
    Medium { title: "Hitchhiker's (DOS 360K)", file: "Hitchhiker's Guide to the Galaxy, The (1987) (r58, Serial 851002) (Infocom, Inc.) (360K) [!].ima", image: Some(DiskImage::Fat12Dos), version: 3, release: 58, serial: "851002" },
    Medium { title: "Hitchhiker's (Lost Treasures)", file: "floppy2.ima", image: Some(DiskImage::Fat12Dos), version: 5, release: 31, serial: "871119" },
    // An Atari ST compilation: four games in four folders, every one of them
    // called `STORY.DAT`, so the conventional-name tiebreak cannot separate them
    // and the largest is what opening the disk gives you — *Bureaucracy* v4 r86.
    // (The other three are Hitchhiker's v3 r56 s841221, Cutthroats v3 r23
    // s840809 and Leather Goddesses v3 r59 s860730; `blorb::medium`'s own suite
    // pins the whole list.)
    Medium { title: "Bureaucracy (Atari ST)", file: "Infocom Compilation 9 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 86, serial: "870212" },
    // The other directoried ST compilation, and the one that gets to PLAY below:
    // four folders again, four more `STORY.DAT`s, and the largest is *Trinity*.
    // (Compilation 9's Bureaucracy opens on its licence form and never reaches
    // an ordinary prompt, which makes it a fine identity pin and a poor smoke.)
    Medium { title: "Trinity (Atari ST)", file: "Infocom Compilation 8 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 4, release: 11, serial: "860509" },
    // **The one story in the ST corpus whose BEHAVIOUR the profile moves.**
    // Compilation 7 in the survey's numbering, flat 8.3 names, and `BEYZORK.T`
    // is by some distance the largest file on it (262144 bytes against ~85K for
    // each Zork), so opening the disk gives you *Beyond Zork* — which is what
    // makes it usable as a plain row here. Every other ST story either ignores
    // `$1E` entirely or merely prints it; this one changes what it draws and
    // what it asks. `atari_st_profile.rs` is where that is measured.
    Medium { title: "Beyond Zork (Atari ST)", file: "Infocom Compilation 6 (19xx)(-).st", image: Some(DiskImage::Fat12AtariSt), version: 5, release: 49, serial: "870917" },
    // ── The Apple II (SQ-0836) ───────────────────────────────────────────────
    //
    // ProDOS media, and the fifth filesystem babelmap mounts. Every row here was
    // measured through `app::hints::load_mounted_story` on 2026-08-13, like the
    // rest of the table.
    //
    // The standalone Apple IIgs *Beyond Zork* — a GS/OS boot disk with `BZ.DAT`
    // on it — is the SAME build as the Amiga floppy and the bare `.z5` beside
    // them, which makes it this corpus's clearest "sometimes they do agree".
    Medium { title: "Beyond Zork (Apple IIgs)", file: "Beyond Zork (1988)(Infocom).2mg", image: Some(DiskImage::ProDos), version: 5, release: 57, serial: "871221" },
    // *The Lost Treasures of Infocom* (1993, Big Red Computer Club) is seven
    // Apple IIgs volumes. Volume 1 is the GS/OS launcher and carries no game at
    // all, so it is absent here; volumes 2–7 carry thirty between them and
    // each row below is the one that OPENS — no ProDOS release has a
    // conventional story name, so the largest wins (`blorb::prodos` pins the
    // full inventory of all seven).
    Medium { title: "Beyond Zork (Lost Treasures IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 2 of 7).2mg", image: Some(DiskImage::ProDos), version: 5, release: 57, serial: "871221" },
    Medium { title: "Stationfall (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 3 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 107, serial: "870430" },
    Medium { title: "The Lurking Horror (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 4 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 203, serial: "870506" },
    // **The row that earns its place.** *Trinity* off the IIgs collection is v4
    // r12 s860926; *Trinity* off `Infocom Compilation 8 (19xx)(-).st`, four rows
    // up, is v4 r11 s860509. Same game, two media, two builds — the project's
    // own rule on a third machine, and now pinned rather than assumed.
    Medium { title: "Trinity (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 5 of 7).2mg", image: Some(DiskImage::ProDos), version: 4, release: 12, serial: "860926" },
    Medium { title: "Sherlock (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 6 of 7).2mg", image: Some(DiskImage::ProDos), version: 5, release: 21, serial: "871214" },
    Medium { title: "Wishbringer (Apple IIgs)", file: "Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg", image: Some(DiskImage::ProDos), version: 3, release: 69, serial: "850920" },
];

/// The pairs, and whether the two media carry the SAME build. Every `false`
/// here is a title where a finding measured off one medium says nothing about
/// the other.
const PAIRS: &[(&str, bool)] = &[
    ("Journey", false),
    ("Zork Zero", false),
    ("Shogun", false),
    ("Arthur", false),
    ("Beyond Zork", true),
    ("Zork I", true),
    ("Zork II", true),
    ("Zork III", true),
];

/// The v6 frame each medium produces after `turns` blank turns from boot, under
/// the profile its own medium resolves to: which window its prose is streaming
/// into, and that window's box as the game set it (x, y, w, h — native pixels,
/// 1-based like the Z-machine's own coordinates).
///
/// The prose window is the load-bearing number: it is the fact SQ-0755 turned
/// on, and the one place where Journey's two releases visibly disagree.
struct V6Frame {
    file: &'static str,
    turns: usize,
    prose_window: usize,
    box_px: (u16, u16, u16, u16),
}

const V6_FRAMES: &[V6Frame] = &[
    // r30 narrates through window 2 — window 0 is a leftover strip it never
    // prints into. r83 narrates through window 0 and has no window 2 at all.
    V6Frame { file: "Journey - The Quest Begins.adf", turns: 40, prose_window: 2, box_px: (265, 17, 368, 272) },
    V6Frame { file: "journey-r83-s890706.z6", turns: 40, prose_window: 0, box_px: (241, 1, 392, 304) },
    // Zork Zero's two builds place the story window two pixels apart and size it
    // four pixels differently — small, and enough to make a geometry finding off
    // one medium untrue of the other.
    V6Frame { file: "Zork Zero - The Revenge of Megaboz.adf", turns: 12, prose_window: 0, box_px: (89, 81, 464, 320) },
    V6Frame { file: "zork0-r393-s890714.z6", turns: 12, prose_window: 0, box_px: (87, 79, 468, 320) },
    // …and the Macintosh disk's r296, on the big colour Mac's 640×400 screen —
    // the same box r393 lays out, which is the point: the Mac's COLOUR archive
    // is the Amiga's picture space (`wx := 2*GFXAM_X`), so the geometry does not
    // move. Only the monochrome archive is a different screen (SQ-0838), and
    // `v6_macintosh_profile.rs` is where that one is pinned.
    V6Frame { file: "Zork Zero Disk.image", turns: 12, prose_window: 0, box_px: (87, 79, 468, 320) },
    // Shogun and Arthur agree across their two builds. Pinned so that stays a
    // measured fact rather than an assumption.
    V6Frame { file: "James Clavell's Shogun.adf", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "shogun-r322-s890706.z6", turns: 12, prose_window: 0, box_px: (47, 33, 548, 368) },
    V6Frame { file: "Arthur - The Quest for Excalibur.adf", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
    V6Frame { file: "arthur-r74-s890714.z6", turns: 12, prose_window: 0, box_px: (1, 1, 640, 400) },
];

/// Media whose game narrates its own release, and the words it uses. The
/// strongest possible statement of which build is running: the STORY says so,
/// not the header we read to pick the fixture.
const NARRATED: &[(&str, &[&str])] = &[
    ("Zork Zero - The Revenge of Megaboz.adf", &["Release 366 / Serial number 890323"]),
    ("zork0-r393-s890714.z6", &["Release 393 / Pix 393 / Serial number 890714"]),
    // The Macintosh disk goes one better than ZTUU: Zork Zero r296 does not
    // print the NUMBER, it prints the MACHINE. "Macintosh Interpreter version
    // 6.65" is the game's own reading of header `$1E`, and it says Macintosh
    // only because SQ-0838 told it 3 — the same disk answered "IBM Interpreter"
    // for as long as an HFS volume resolved to the IBM PC default.
    ("Zork Zero Disk.image", &["Macintosh Interpreter version", "Release 296 / Serial number 881019"]),
    ("James Clavell's Shogun.adf", &["Release 295 / Serial number 890321"]),
    ("shogun-r322-s890706.z6", &["Release 322 / Pix 322 / Serial number 890706"]),
    ("Zork I - The Great Underground Empire.adf", &["Revision 88 / Serial number 840726"]),
    ("zork1-r88-s840726.z3", &["Revision 88 / Serial number 840726"]),
    ("Zork II - The Wizard of Frobozz.adf", &["Version 48 / Serial number 840904"]),
    ("Zork III - The Dungeon Master.adf", &["Release 17 / Serial number 840727"]),
    // …and ZTUU prints the interpreter it was told it is running on, which off a
    // floppy is the Amiga's 4 (ZMSD §11.1.3). One line proving both halves of
    // "the medium picks the machine".
    ("Zork - The Undiscovered Underground.adf", &["Release 16 / Serial number 970828", "Interpreter 4 "]),
    // **The PC and the Atari ST open and play** (SQ-0833, SQ-0835). Mounting is
    // half a claim; this is the other half — the game boots off the floppy,
    // reaches its prompt, and names the release the disk carries. Zork Zero's
    // DOS press draws its own EGA art off the same disk on the way.
    ("floppy5.ima", &["Release 393 / Pix 393 / Serial number 890714"]),
    // Trinity off the ST floppy prints its interpreter number too, and **this is
    // the line SQ-0835 left here to be changed.** It read `Interpreter 1 ` for
    // one commit, while the container read an ST floppy and then ran it as a
    // DECSystem-20; it now reads 5, ZMSD §11.1.3's Atari ST, which is also what
    // Infocom's own ST interpreters write into `$1E` (`INTWRD DC.B 5`).
    //
    // Trinity is a Version 4 story, which is the interesting half: byte `$1E`
    // means nothing before Version 4, and the ST's own Version 3 build leaves it
    // zero and comments it "(UNUSED)". So this is a story that genuinely reads
    // the byte, and it reports the machine it is now correctly told it is on.
    ("Infocom Compilation 8 (19xx)(-).st", &["Release 11 / Serial Number 860509", "Interpreter 5 "]),
    // **And the Apple II press opens and plays** (SQ-0836). Mounting a ProDOS
    // volume, walking its directory tree and pulling a story out of a tree file
    // is half a claim; this is the other half — the game boots off the disk
    // image, reaches its prompt, and names the release the disk carries.
    //
    // Two of them, because the interesting pair is *Trinity*: the line below is
    // r12 s860926 off the IIgs collection, where the Atari ST row above prints
    // r11 s860509 for the same game. Same title, two floppies, two builds, and
    // both are now the story's own word rather than a header we read.
    ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 5 of 7).2mg", &["Release 12 / Serial Number 860926"]),
    ("Lost Treasures of Infocom, The (1993)(Big Red Computer Club)(Disk 7 of 7).2mg", &["Release 69 / Serial Number 850920"]),
];

/// The resource Blorbs shipped beside these stories, and whether each declares
/// a `Reso` standard window. None of them carries an executable chunk, so a
/// Blorb is never a third build — it is artwork for whichever story file you
/// point at, and the release stays the one you opened.
const RESOURCE_BLORBS: &[(&str, bool)] = &[
    ("Journey.blb", true),
    ("Zork0.blb", true),
    ("Shogun.blb", true),
    ("Arthur.blb", true),
    // Beyond Zork is a v5 story: its Blorb carries sound, no scalable pictures
    // and so no standard window at all.
    ("beyondzork.blb", false),
];

/// Pane widths the render case sweeps. 80 is the game's own column count, where
/// the cell path's 1:1 chrome and the proportional placement coincide and a
/// defect can hide (SQ-0742's lesson); the other two do not coincide.
const WIDTHS: [u16; 3] = [80, 100, 138];

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Is ANY of this suite's media present? `stories/` is gitignored, so CI and a
/// fresh worktree legitimately have none and every case here skips. This gates
/// the `ran > 0` guards so they only ever catch a drifted filename on a machine
/// that really does hold the media.
fn any_real_media_present() -> bool {
    MEDIA.iter().any(|m| stories_dir().join(m.file).exists())
}

/// How every assertion in this file names its subject. A failure that does not
/// say which release it loaded is the single most expensive kind of failure this
/// project has had.
fn ctx(m: &Medium) -> String {
    format!(
        "{} [{} — {}, release {}, serial {}]",
        m.file,
        m.title,
        match m.image {
            Some(DiskImage::Adf) => "Amiga floppy",
            Some(DiskImage::Hfs) => "Macintosh floppy",
            Some(DiskImage::Fat12Dos) => "DOS floppy",
            Some(DiskImage::Fat12AtariSt) => "Atari ST floppy",
            Some(DiskImage::ProDos) => "Apple ProDOS floppy",
            None => "story file",
        },
        m.release,
        m.serial
    )
}

/// The story bytes off `m`'s medium, or `None` (with a SKIP note) when the
/// gitignored fixture is not there.
fn story_bytes(m: &Medium) -> Option<Vec<u8>> {
    let path = stories_dir().join(m.file);
    match app::hints::load_mounted_story(&path) {
        Ok((loaded, mounted)) => {
            assert_eq!(
                mounted,
                m.image,
                "{}: the mount reports the medium, and it disagrees with the table",
                ctx(m)
            );
            Some(loaded.bytes().to_vec())
        }
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            None
        }
    }
}

fn header_release(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[2], bytes[3]])
}

fn header_serial(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned()
}

/// The guard every case runs before it measures anything: the bytes in hand are
/// the build the table names. Nothing measured after this can be attributed to
/// the wrong release.
fn assert_is_the_pinned_release(m: &Medium, bytes: &[u8]) {
    assert_eq!(bytes[0], m.version, "{}: Z-machine version", ctx(m));
    assert_eq!(
        header_release(bytes),
        m.release,
        "{}: loaded release {} — this medium carries a DIFFERENT build than the table says",
        ctx(m),
        header_release(bytes)
    );
    assert_eq!(
        header_serial(bytes),
        m.serial,
        "{}: loaded serial {}",
        ctx(m),
        header_serial(bytes)
    );
}

/// Boot `m` exactly as `startup.rs` does — the profile comes from the medium,
/// the artwork from whatever that medium supplies — after checking the build.
fn boot(m: &Medium, honor_game_colours: bool) -> Option<GameSession> {
    let bytes = story_bytes(m)?;
    assert_is_the_pinned_release(m, &bytes);
    let path = stories_dir().join(m.file);
    let profile = InterpreterProfile::resolve(&path, None, None);
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    // The same chain, in the same order: the Blorb's `Reso`, the archive the
    // medium supplied, then the machine (SQ-0838). No medium here names an
    // archive by hand, so tier 3's link is the one that is absent.
    let v6_screen_px = picts
        .std_window()
        .or_else(|| picts.native_std_window())
        .or_else(|| profile.std_window());
    let mut s = GameSession::new_with_art_scale(
        bytes,
        honor_game_colours,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        picts.art_scale(),
        profile.default_colours(),
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(m)));
    assert!(!s.quit, "{}: quit during boot", ctx(m));
    assert!(s.machine.fault_trace.is_none(), "{}: faulted during boot", ctx(m));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive `turns` turns of "just keep going": Enter at a keypress, an empty line
/// at a line prompt. Deterministic, which is what a pinned frame needs.
fn drive(s: &mut GameSession, turns: usize, who: &str) {
    for t in 0..turns {
        match s.pending_input() {
            InputKind::Line => {
                let _ = s.submit("");
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
        assert!(!s.quit, "{who}: quit at turn {t} of {turns}");
        assert!(s.machine.fault_trace.is_none(), "{who}: faulted at turn {t} of {turns}");
    }
}

/// The window this session's prose is currently streaming into, and its box as
/// the game set it. `None` for a non-v6 story, or when nothing has been printed.
fn prose_window(s: &GameSession) -> Option<(usize, (u16, u16, u16, u16))> {
    let v6 = s.machine.screen.v6.as_ref()?;
    v6.windows
        .iter()
        .enumerate()
        .find(|(_, w)| !w.streamed.is_empty())
        .map(|(i, w)| (i, (w.x_coord, w.y_coord, w.x_size, w.y_size)))
}

/// Which render path drew this frame, as `/dump-windows` reports it.
fn path_label(state: &app::state::AppState) -> String {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label.starts_with("path:"))
        .map(|e| e.label.clone())
        .unwrap_or_else(|| "<no path recorded>".into())
}

/// The native-pixel rect the renderer treated as the STORY viewport this frame.
fn viewport_px(state: &app::state::AppState) -> Option<(u16, u16, u16, u16)> {
    state.v6_cell_map.borrow().iter().find(|e| e.label == "viewport").map(|e| e.native)
}

// ── Identity ─────────────────────────────────────────────────────────────────

/// The table itself, checked against the media. This is the finding SQ-0760
/// exists to record: what each medium in `stories/` actually contains.
#[test]
fn every_medium_loads_the_release_it_is_pinned_at() {
    let mut ran = 0;
    for m in MEDIA {
        let Some(bytes) = story_bytes(m) else { continue };
        assert_is_the_pinned_release(m, &bytes);
        ran += 1;
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but none were read");
}

/// The headline. For every title shipping on both media, the two releases are
/// pinned against **each other** — so a future agent cannot quietly measure one
/// and report it as the other, and an upgrade of either fixture fails here
/// rather than silently rebasing an investigation.
#[test]
fn a_floppy_and_the_story_file_beside_it_are_pinned_against_each_other() {
    let mut ran = 0;
    for (title, same_build) in PAIRS {
        let pair: Vec<&Medium> = MEDIA.iter().filter(|m| m.title == *title).collect();
        assert_eq!(pair.len(), 2, "{title}: PAIRS names a title that is not a pair in MEDIA");
        let (floppy, file) = (pair[0], pair[1]);
        assert!(
            floppy.image.is_some() && file.image.is_none(),
            "{title}: MEDIA must list the floppy first"
        );
        let (Some(fb), Some(sb)) = (story_bytes(floppy), story_bytes(file)) else { continue };
        assert_is_the_pinned_release(floppy, &fb);
        assert_is_the_pinned_release(file, &sb);
        ran += 1;

        let same = header_release(&fb) == header_release(&sb)
            && header_serial(&fb) == header_serial(&sb);
        assert_eq!(
            same, *same_build,
            "{title}: the floppy is release {} serial {} and the story file is release {} serial {} \
             — {}. A finding measured on one medium describes the other ONLY when these agree.",
            header_release(&fb),
            header_serial(&fb),
            header_release(&sb),
            header_serial(&sb),
            if *same_build { "they were pinned as the same build" } else { "they were pinned as different builds" },
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no pair was compared");
}

/// **Every medium here is one the story picker actually OFFERS** (SQ-0849).
///
/// The report: *"i don't see any games with ima or st extensions in the story
/// list."* They were not there. Mounting worked — every case above proves it —
/// but the picker's directory scan pre-filtered on its own hand-written
/// extension list, which knew `.adf` and `.image` and had never heard of the DOS
/// and Atari ST rows SQ-0833 and SQ-0835 added. Those floppies were opened by
/// name and invisible in the list, which is the only place most people look.
///
/// So this drives `picker::scan_stories` over the real story directory and
/// insists that every medium in [`MEDIA`] that exists on disk comes back out of
/// it. The pre-filter now derives from `blorb::medium`'s format table, so a
/// format added there is offered here in the same commit.
///
/// `stories/` is gitignored, so this skips vacuously per missing file and the
/// `ran > 0` guard is gated on [`any_real_media_present`], exactly like its
/// neighbours — CI has no media at all and must not fail on their absence.
///
/// Measured on the local corpus, 2026-08-13: the scan listed **142** stories
/// before and **163** after — 8 `.ima`, 4 `.img` and 9 `.st` that were mountable
/// all along. The `.adf` count (9) and the `.image` count (1) did not move, and
/// three `.ima` in the directory are still *not* listed, because no story comes
/// out of them: the extension is a pre-filter, not a verdict.
///
/// FALSIFICATION: restore the disk spellings as a literal `"adf", "image"` in
/// `picker::STORY_EXTS` and this fails naming `floppy5.ima` — the user's symptom,
/// verbatim.
#[test]
fn every_release_medium_is_offered_by_the_story_picker() {
    let dir = stories_dir();
    let data_base = std::env::temp_dir().join(format!("babelmap-sq0849-{}", std::process::id()));
    let listed: Vec<PathBuf> =
        app::picker::scan_stories(&dir, &data_base).into_iter().map(|e| e.path).collect();
    let _ = std::fs::remove_dir_all(&data_base);

    let mut ran = 0;
    for m in MEDIA {
        let path = dir.join(m.file);
        if !path.is_file() {
            continue;
        }
        ran += 1;
        assert!(
            listed.contains(&path),
            "{} is mountable but the picker never offered it",
            ctx(m)
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but none were scanned");

    // The one format with no reader yet stays out of the list, however many of
    // them sit in the directory: Apple II 5.25" `.dsk` (SQ-0852) arrives with
    // the code that opens it, not before.
    let queued = "dsk";
    assert!(
        !listed.iter().any(|p| {
            p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(queued))
                == Some(true)
        }),
        "a .{queued} was listed, but nothing in blorb can mount one"
    );

    // …and `.2mg` moved from that list to this one in the same commit as its
    // reader (SQ-0836), which is the whole point of the extension column. The
    // two ProDOS images that are still absent are absent for the right reason:
    // `Arthur Quest 4 Excalibur.2mg` and `Journey.2mg` are the segmented Apple
    // II press and no whole story comes out of them, so the pre-filter opened
    // them and the mount declined to offer one. The extension is a pre-filter,
    // not a verdict.
    let dir_has_2mg = std::fs::read_dir(&dir).into_iter().flatten().flatten().any(|e| {
        e.path().extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("2mg"))
            == Some(true)
    });
    let listed_2mg: Vec<&std::ffi::OsStr> = listed
        .iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("2mg"))
                == Some(true)
        })
        .filter_map(|p| p.file_name())
        .collect();
    assert_eq!(
        !listed_2mg.is_empty(),
        dir_has_2mg,
        "ProDOS media are in the directory and the picker offered none of them"
    );
    for absent in ["Arthur Quest 4 Excalibur.2mg", "Journey.2mg"] {
        assert!(
            !listed_2mg.iter().any(|n| *n == std::ffi::OsStr::new(absent)),
            "{absent} carries no whole story file and must not be offered as one"
        );
    }
}

/// A `.blb` beside a story is artwork, never a third build — so "which release"
/// is decided entirely by the file you open.
#[test]
fn a_resource_blorb_beside_a_story_carries_no_release_of_its_own() {
    let mut ran = 0;
    for (name, declares_std_window) in RESOURCE_BLORBS {
        let path = stories_dir().join(name);
        if !path.exists() {
            eprintln!("SKIP: gitignored resource Blorb missing at {}", path.display());
            continue;
        }
        ran += 1;
        assert!(
            app::hints::load_story(&path).is_err(),
            "{name}: a resource Blorb must hold no executable, or it would be a third build"
        );
        let std_window =
            PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b)).std_window();
        assert_eq!(
            std_window.is_some(),
            *declares_std_window,
            "{name}: standard window is {std_window:?}"
        );
        if *declares_std_window {
            // …and it is the SAME 320×200 the Amiga profile supplies for a
            // native archive, which has no field to declare one (SQ-0736).
            assert_eq!(
                std_window,
                InterpreterProfile::Amiga.std_window(),
                "{name}: an Infocom Blorb's Reso declares the machine's own standard window"
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no Blorb was read");
}

/// The medium picks the machine, on the real files rather than a synthetic disk
/// (`interpreter_profile.rs` covers the synthetic side). This is the rule that
/// makes "the Amiga build" ambiguous in the first place, so it is pinned on the
/// same fixtures every other case here uses.
#[test]
fn the_medium_each_release_ships_on_picks_the_interpreter_profile() {
    let mut ran = 0;
    for m in MEDIA {
        let path = stories_dir().join(m.file);
        if !path.exists() {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            continue;
        }
        ran += 1;
        // SQ-0838 lifted the Macintosh's block: an HFS volume now resolves to
        // the Macintosh bundle (interpreter 3, black on white, a screen sized by
        // the archive it carries), all of it read out of Infocom's own Mac
        // interpreter rather than out of anyone's memory. Everything that is not
        // release media is still the IBM PC default.
        let expected = match m.image {
            Some(DiskImage::Adf) => InterpreterProfile::Amiga,
            Some(DiskImage::Hfs) => InterpreterProfile::Macintosh,
            // A DOS floppy IS the IBM PC, and the IBM PC bundle is the one that
            // deliberately announces no number of its own (SQ-0833) — so this
            // arm and the fall-through below reach the same profile by design
            // rather than by accident.
            Some(DiskImage::Fat12Dos) => InterpreterProfile::IbmPc,
            // **And the Atari ST is now its own machine** (SQ-0835's profile
            // half): interpreter 5, black on white, §8.3.1's palette, and no
            // standard window, because Infocom never wrote a Version 6
            // interpreter for the ST. Read this arm against the one above it —
            // same FAT12 filesystem, different machine, different answer.
            Some(DiskImage::Fat12AtariSt) => InterpreterProfile::AtariSt,
            // **And a ProDOS volume IS its own machine after all** (SQ-0857,
            // reversing SQ-0836 — this is the one row this quest deliberately
            // moved). ProDOS still names the Apple II *family*, and §11.1.3 still
            // numbers three machines in it (2 IIe, 9 IIc, 10 IIgs) — Infocom's
            // own Apple II YZIP picks between all three by DETECTING the machine
            // at boot, and that routine is byte-for-byte on `Journey.2mg` and
            // `Arthur Quest 4 Excalibur.2mg`. What reversed the conclusion is
            // that declining is not neutral: it lands an Apple II story on zvm's
            // Frotz rule, which names the DECSystem-20 or, on Version 6, the IBM
            // PC. §11.1.3 asks for "the machine it will run on", and of the three
            // the YZIP starts on, that is the IIgs. Argued in full at the row in
            // `blorb::medium`, with the thirty-story trace measurement behind it.
            Some(DiskImage::ProDos) => InterpreterProfile::AppleIIgs,
            None => InterpreterProfile::IbmPc,
        };
        assert_eq!(
            InterpreterProfile::resolve(&path, None, None),
            expected,
            "{}: the medium decides the profile",
            ctx(m)
        );
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no profile was resolved");
}

// ── What each medium actually does ───────────────────────────────────────────

/// Every v6 medium boots and reaches a stable frame, and that frame is pinned
/// per medium — in BOTH `honor_game_colours` modes, per the project's colour
/// convention (`true` is the shipped default).
///
/// Journey is the case with teeth: the floppy's r30 streams its prose into
/// window 2 and the story file's r83 into window 0.
#[test]
fn each_v6_medium_narrates_through_the_window_its_own_release_uses() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let found = prose_window(&s)
                .unwrap_or_else(|| panic!("{who}: nothing was streamed after {} turns", f.turns));
            assert_eq!(
                found.0, f.prose_window,
                "{who}: this release narrates through window {}, not {} — a rule aimed at the \
                 wrong window is right by luck on one release and wrong on the other (SQ-0755)",
                found.0, f.prose_window
            );
            assert_eq!(
                found.1, f.box_px,
                "{who}: window {} is at {:?}, pinned at {:?} (native px, 1-based)",
                f.prose_window, found.1, f.box_px
            );
            ran += 1;
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no v6 frame was measured");
}

/// The games that print their own release print the one that was loaded — the
/// story's word, not ours. Off the ZTUU floppy that line also names interpreter
/// 4, so the medium's profile is visible in the game's own output.
#[test]
fn a_game_that_names_its_release_names_the_one_the_medium_carries() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for (file, wanted) in NARRATED {
        let m = MEDIA.iter().find(|m| m.file == *file).expect("NARRATED names a medium in MEDIA");
        let Some(mut s) = boot(m, true) else { continue };
        ran += 1;
        // Past whatever intro the release opens with, to its first line prompt.
        let mut said = String::new();
        for _ in 0..24 {
            match s.pending_input() {
                InputKind::Line => {
                    said = s.submit("version").transcript;
                    break;
                }
                InputKind::Char => {
                    let _ = s.submit_char(13);
                }
                InputKind::Event => {
                    let _ = s.submit("");
                }
            }
        }
        let flat = said.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in *wanted {
            assert!(
                flat.contains(want),
                "{}: the game answered VERSION with {flat:?}, which does not say {want:?}",
                ctx(m)
            );
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no game was asked");
}

/// A copy of a release floppy in a directory of its own, deleted when it drops.
///
/// **The isolation is the point, not tidiness.** `stories/` already holds loose
/// `zork0.eg1`, `zork0.cg1` and `zork0.mg1` fixtures that came off these very
/// disks, and `PictureOverride` tries the host filesystem before the medium — so
/// a test that names `ZORK0.EG1` beside them proves nothing about the medium at
/// all, and on a case-insensitive filesystem does not even fail loudly. It
/// passed under a deliberately broken mount before this struct existed.
struct FloppyAlone {
    dir: PathBuf,
    image: PathBuf,
}

impl Drop for FloppyAlone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn floppy_alone(tag: &str) -> Option<FloppyAlone> {
    let src = stories_dir().join("floppy5.ima");
    if !src.is_file() {
        eprintln!("SKIP: gitignored medium missing at {}", src.display());
        return None;
    }
    let dir = std::env::temp_dir().join(format!("babelmap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory of its own");
    let image = dir.join("floppy5.ima");
    std::fs::copy(&src, &image).expect("the floppy is copied");
    Some(FloppyAlone { dir, image })
}

/// A release floppy supplies its own artwork, on **every** machine that pressed
/// one — and the PC's is a different codec from the Amiga's and the Mac's
/// (SQ-0833).
///
/// This is the pairing the medium guarantees and no configuration has to: open
/// `floppy5.ima` and *Zork Zero*'s story and its EGA art come off the same disk,
/// through the same mount, with nothing named by hand. `ZORK0.EG1` is the
/// LZW-coded PC archive where the Amiga and Macintosh floppies hand back the
/// big-endian Huffman one — `blorb::fat12` pins the flavour, and this pins that
/// the app reaches it and can draw from it.
///
/// It is also where the one-image limit is visible: this disk carries the EGA
/// rendition and *Zork Zero*'s CGA rendition is on floppy4, which this mount
/// cannot reach. A set model would; see the module docs of `blorb::fat12`.
#[test]
fn a_dos_release_floppy_supplies_its_own_pc_artwork() {
    let Some(disk) = floppy_alone("dos-art") else { return };
    let mut picts = PictSource::resolve(&disk.image);
    let dims = picts.all_pict_dims();
    assert!(dims.len() > 100, "the disk's own archive, {} pictures", dims.len());
    assert!(
        picts.image(1).is_some(),
        "picture 1 decodes straight off the floppy, with no extraction step"
    );
}

/// **Offered and openable, which are two claims** (SQ-0833 + SQ-0843).
///
/// The launch dialog enumerates a disk's files through `assets::files`, which
/// asks `blorb::medium` and therefore gained FAT12 for free; the `--pictures`
/// door then loads the chosen one through `PictureOverride`. Those were two
/// different code paths, and the second one still carried a hand-written
/// `looks_like_adf … else if looks_like_hfs` chain — so a DOS disk's `ZORK0.EG1`
/// would have appeared in the list and drawn nothing when picked.
///
/// FALSIFICATION: restore that chain in `graphics::read_off_the_medium` and this
/// fails at the `Loaded` assertion with `Missing` — the file the same disk had
/// just listed.
#[test]
fn artwork_on_a_dos_floppy_is_both_listed_and_loadable() {
    let Some(disk) = floppy_alone("dos-listed") else { return };
    let listed: Vec<String> = app::assets::files(&disk.image)
        .into_iter()
        .filter(app::assets::AssetFile::is_on_medium)
        .map(|f| f.name)
        .collect();
    assert_eq!(listed, ["ZORK0.EG1", "ZORK0.ZIP"], "the disk's own files, in disk order");

    // Every name the dialog can show has to be a name the door accepts.
    let over =
        app::graphics::PictureOverride::resolve_with_session(&disk.image, &disk.dir, Some("ZORK0.EG1"));
    assert!(
        matches!(over, app::graphics::PictureOverride::Loaded { .. }),
        "naming the disk's own archive loads it: {over:?}"
    );
    assert_eq!(
        over.flavour(),
        Some(blorb::infocom_pics::Flavour::Pc),
        "…and it is the PC's LZW archive, so the machine stays the IBM PC"
    );
}

/// …and the other half of the rule, on the same real floppy: an explicitly
/// requested interpreter number BEATS the medium (SQ-0839).
///
/// The ordering is a contract, not an implementation detail — the medium only
/// ever moves the DEFAULT — and it is worth proving where the game can be heard
/// saying so rather than only at `resolve`. ZTUU's Inform banner prints header
/// `$1E` outright, so asking for the IBM PC's 6 off an Amiga floppy is audible.
/// `zvm-cli`'s `disk_image` suite pins the identical pair through the CLI.
///
/// FALSIFICATION: reorder `InterpreterProfile::resolve` to consult the medium
/// before the explicit number and the game answers `Interpreter 4`, ignoring
/// what was asked for.
#[test]
fn an_explicit_interpreter_number_outranks_the_floppy_it_was_opened_from() {
    const ZTUU: &str = "Zork - The Undiscovered Underground.adf";
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let m = MEDIA.iter().find(|m| m.file == ZTUU).expect("ZTUU is in MEDIA");
    let Some(bytes) = story_bytes(m) else { return };
    assert_is_the_pinned_release(m, &bytes);

    // The medium says Amiga; the launch says IBM PC. The launch wins, and it
    // brings its whole profile with it — that is what asking for a number means.
    let path = stories_dir().join(m.file);
    let profile = InterpreterProfile::resolve(&path, Some(6), None);
    assert_eq!(profile, InterpreterProfile::IbmPc, "{}: explicit beats the medium", ctx(m));
    zvm::screen::set_palette(profile.palette());

    let mut s = GameSession::new_with_trace(
        bytes,
        true,
        false,
        // The IBM PC profile has no opinion, so the launch's own number is what
        // reaches the header — exactly as `startup.rs` composes it.
        profile.interpreter_number().or(Some(6)),
        false,
        Vec::new(),
        profile.std_window(),
        profile.default_colours(),
        None,
    )
    .unwrap_or_else(|e| panic!("{}: should boot without a ZError: {e:?}", ctx(m)));

    let mut said = String::new();
    for _ in 0..24 {
        match s.pending_input() {
            InputKind::Line => {
                said = s.submit("version").transcript;
                break;
            }
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    let flat = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("Interpreter 6 "),
        "{}: asked for interpreter 6 off an Amiga floppy, and the game answered {flat:?}",
        ctx(m)
    );
}

/// …and the floppy build reaches the RENDERER, which is where its differently
/// placed windows could be dropped on the floor. Nothing committed had ever
/// rendered a frame produced by a disk image at all.
///
/// Two things per medium, at three pane widths and in both colour modes: the
/// frame takes the ordinary gameplay path (a release whose chrome trips the
/// painted-menu gate would be routed elsewhere — SQ-0742), and the story
/// viewport the renderer picks out is the box of the window THIS release
/// narrates through. On the Journey floppy that is window 2; on the story file
/// beside it, window 0.
#[test]
fn each_v6_medium_renders_the_story_viewport_its_own_release_lays_out() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut ran = 0;
    for f in V6_FRAMES {
        let m = MEDIA.iter().find(|m| m.file == f.file).expect("frame names a medium in MEDIA");
        for honor in [true, false] {
            let Some(mut s) = boot(m, honor) else { continue };
            let who = format!("{} honor_game_colours={honor}", ctx(m));
            drive(&mut s, f.turns, &who);
            let model = s.screen();
            // The window table is 1-based (ZMSD §8.8.1); the renderer works in
            // 0-based screen pixels, so the story viewport is that box shifted.
            let want = (f.box_px.0 - 1, f.box_px.1 - 1, f.box_px.2, f.box_px.3);
            for cols in WIDTHS {
                let state = render_hybrid(&model, honor, cols, 51);
                assert_eq!(
                    path_label(&state),
                    "path:hybrid-ring",
                    "{who} w={cols}: an ordinary gameplay frame must take the chrome ring"
                );
                assert_eq!(
                    viewport_px(&state),
                    Some(want),
                    "{who} w={cols}: the story viewport must be window {}'s box",
                    f.prose_window
                );
                ran += 1;
            }
        }
    }
    assert!(ran > 0 || !any_real_media_present(), "media are present but no frame was rendered");
}

/// A hybrid render at a plausible kitty cell (8×18); `Picker::halfblocks()`
/// reports a 1×2 cell, a layout regime that reproduces nothing (SQ-0548).
#[allow(deprecated)]
fn render_hybrid(
    model: &app::engine::ScreenModel,
    honor: bool,
    cols: u16,
    rows: u16,
) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    let area = Rect::new(0, 0, cols, rows);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(model, false, None, &state, area, &mut buf);
    state
}
