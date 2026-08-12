//! The Amiga palette and default pair, read back out of the interpreters on
//! Infocom's own release floppies — SQ-0822.
//!
//! babelmap's Amiga profile came from `amiga/yzip1.c` and `amiga/yzip.h` in the
//! leaked historical interpreter sources. Those are a DEVELOPMENT snapshot, and
//! they disagree with what Infocom actually shipped in two places — one of which
//! is the whole screen:
//!
//! | constant   | `yzip*.h/.c` | on the floppy | what it is           |
//! |------------|--------------|---------------|----------------------|
//! | `DEF_BACK` | 11 (`$777`)  | **12** (`$444`) | the page every game is played on |
//! | `colortable[5]` | `$0EE0` | **`$0FD0`**   | standard colour 5, yellow |
//!
//! The report that opened this quest was the first: Arthur's page came out
//! medium grey where a real Amiga shows dark grey, and a screenshot of the church
//! scene measures `#444444` under `#FFFFFF` — with the status bar reversed to
//! `#444444` on `#FFFFFF`, which is pens 0 and 1 swapped and therefore proof that
//! the page is the text background REGISTER and not artwork.
//!
//! **A unit test that shares the implementation's wrong assumption passes anyway**
//! (CLAUDE.md), and `crates/zvm/tests/amiga_palette.rs` is exactly such a test —
//! it transcribes the same table it checks. So this suite does not transcribe
//! anything. It mounts each Amiga release floppy, extracts the 68000 interpreter
//! binary Infocom shipped on it, and reads the constants out of that program's own
//! bytes. If a later reader "corrects" `AMIGA_DEFAULT_BACKGROUND` back to 11 on the
//! strength of `yzip.h`, this is the test that will tell them the machine disagrees.
//!
//! `stories/` is gitignored, so a missing floppy skips vacuously — loudly, on
//! stderr.

use std::path::PathBuf;

use app::interpreter::{AMIGA_DEFAULT_BACKGROUND, AMIGA_DEFAULT_FOREGROUND};

/// One Amiga release floppy and the name of the interpreter binary on it. The
/// release/serial are the story's, pinned in `real_media_releases.rs`; the
/// interpreter is a separate program on the same disk.
struct Floppy {
    image: &'static str,
    terp: &'static str,
    release: u16,
}

const FLOPPIES: [Floppy; 4] = [
    Floppy { image: "Arthur - The Quest for Excalibur.adf", terp: "Arthur", release: 54 },
    Floppy { image: "Journey - The Quest Begins.adf", terp: "Journey", release: 30 },
    Floppy { image: "Zork Zero - The Revenge of Megaboz.adf", terp: "Zork Zero", release: 366 },
    Floppy { image: "James Clavell's Shogun.adf", terp: "Shogun", release: 295 },
];

/// The interpreter binary off one floppy, or `None` when the gitignored medium is
/// absent.
fn interpreter(f: &Floppy) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(f.image);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored medium missing at {}", path.display());
        return None;
    };
    let adf = blorb::adf::Adf::mount(bytes).expect("an AmigaDOS disk image");
    let terp = adf.read_named(f.terp).unwrap_or_else(|| {
        panic!(
            "{} (release {}): no interpreter named {:?} on the image; files: {:?}",
            f.image,
            f.release,
            f.terp,
            adf.files().iter().map(|e| e.name.as_str()).collect::<Vec<_>>()
        )
    });
    Some(terp)
}

/// Every occurrence of `needle` in `hay`, as byte offsets.
fn find_all(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    (0..hay.len().saturating_sub(needle.len()) + 1)
        .filter(|&i| &hay[i..i + needle.len()] == needle)
        .collect()
}

/// `colortable[]` as Infocom compiled it: eleven big-endian `$0RGB` words, the
/// first three ordered to match the Workbench defaults. Located by its own
/// content — the two entries this suite is about are read out of the match, never
/// searched for — so a table that moved would still be found and a table that
/// CHANGED would fail loudly instead of silently not matching.
///
/// The anchor is the run either side of the disputed slot 5, which is unique in
/// every binary: blue, white, black, green, red … magenta, cyan, light/med/dark grey.
const CT_HEAD: [u16; 5] = [0x005A, 0x0FFF, 0x0000, 0x00C0, 0x0E00];
const CT_TAIL: [u16; 5] = [0x0F0F, 0x00EE, 0x0AAA, 0x7777, 0x0444];

fn be_words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// ZMSD colour number → `colortable[]` slot, i.e. Infocom's `colormap[]`. Read out
/// of the binary too (below), not trusted from here — this is only the shape.
const COLORMAP: [(u8, usize); 11] =
    [(2, 2), (3, 4), (4, 3), (5, 5), (6, 0), (7, 6), (8, 7), (9, 1), (10, 8), (11, 9), (12, 10)];

/// The 4-bit-per-channel `$0RGB` word, widened to the Z-machine's 15-bit
/// true-colour word by bit replication — the same derivation `amiga_true_colour`
/// documents, written out here independently so a typo cannot agree with itself.
fn widen(rgb444: u16) -> u16 {
    let w = |n: u16| (n << 1) | (n >> 3);
    let (r, g, b) = ((rgb444 >> 8) & 0xF, (rgb444 >> 4) & 0xF, rgb444 & 0xF);
    (w(b) << 10) | (w(g) << 5) | w(r)
}

/// `zvm::screen::amiga_true_colour` IS the table Infocom shipped — every entry,
/// read out of the program that loaded it into the hardware.
///
/// FALSIFY by putting slot 5 back to the development source's `$0EE0`: colour 5
/// fails with "yellow", and nothing else moves.
#[test]
fn the_amiga_palette_is_the_one_compiled_into_the_release_floppies() {
    for f in &FLOPPIES {
        let Some(terp) = interpreter(f) else { return };
        let who = format!("{} [release {}, interpreter {:?}]", f.image, f.release, f.terp);

        let head = find_all(&terp, &be_words(&CT_HEAD));
        assert_eq!(head.len(), 1, "{who}: colortable[0..5] should occur exactly once, at {head:?}");
        let at = head[0];
        let tail = at + 2 * (CT_HEAD.len() + 1);
        assert_eq!(
            &terp[tail..tail + 2 * CT_TAIL.len()],
            &be_words(&CT_TAIL)[..],
            "{who}: colortable[6..11] must follow slot 5 — the table's shape has changed",
        );

        // Slot 5 is whatever the binary says it is. Nothing above pinned it.
        let slot5 = u16::from_be_bytes([terp[at + 10], terp[at + 11]]);
        let table: Vec<u16> = CT_HEAD.iter().copied().chain([slot5]).chain(CT_TAIL).collect();

        // …and `colormap[]` sits immediately after it: two `-1` placeholders and
        // the eleven slot numbers, one per colour, exactly as `COLORMAP` says.
        let cm_at = at + 2 * table.len();
        let mut cm = vec![0xFFu8, 0xFF];
        cm.extend(COLORMAP.iter().map(|&(_, slot)| slot as u8));
        assert_eq!(
            &terp[cm_at..cm_at + cm.len()],
            &cm[..],
            "{who}: colormap[] must follow colortable[] and map colour→slot as assumed",
        );

        for (n, slot) in COLORMAP {
            let raw = table[slot] & 0x0FFF; // SetRGB4 sees only the low 12 bits
            assert_eq!(
                zvm::screen::amiga_true_colour(n),
                Some(widen(raw)),
                "{who}: standard colour {n} is colortable[{slot}] = ${raw:03X}",
            );
        }
    }
}

/// `DEF_FORE` and `DEF_BACK`, out of the two routines that use them.
///
/// `amiga/yzip3.c` opens both with the same line — `if (id == 1) id = DEF_x;`,
/// "colour 1 means the default" — and Lattice C compiled each to `cmpi.w #1,d7`
/// / `bne.s .+2` / `moveq #n,d7`. That three-instruction shape occurs exactly
/// twice in each interpreter: `set_fore` first, then `set_back`, in source order.
///
/// FALSIFY by restoring `AMIGA_DEFAULT_BACKGROUND = 11` (the value in the leaked
/// `amiga/yzip.h`): this fails with "the page the machine boots on".
#[test]
fn the_amiga_default_pair_is_white_on_dark_grey_on_every_release_floppy() {
    for f in &FLOPPIES {
        let Some(terp) = interpreter(f) else { return };
        let who = format!("{} [release {}, interpreter {:?}]", f.image, f.release, f.terp);

        // 0c 47 00 01   cmpi.w #1,d7
        // 66 02         bne.s  .+2
        // 7e ??         moveq  #??,d7
        let hits: Vec<usize> = find_all(&terp, &[0x0C, 0x47, 0x00, 0x01, 0x66, 0x02, 0x7E])
            .into_iter()
            .filter(|i| i % 2 == 0)
            .collect();
        assert_eq!(
            hits.len(),
            2,
            "{who}: set_fore/set_back's `if (id == 1) id = DEF_x` should occur twice, at {hits:?}",
        );
        assert_eq!(
            terp[hits[0] + 7],
            AMIGA_DEFAULT_FOREGROUND,
            "{who}: set_fore's DEF_FORE — the ink the machine boots on",
        );
        assert_eq!(
            terp[hits[1] + 7],
            AMIGA_DEFAULT_BACKGROUND,
            "{who}: set_back's DEF_BACK — the page the machine boots on",
        );

        // Corroboration from a third site, compiled from a different expression:
        // `set_color()` ends `return ((DEF_BACK << 8) | DEF_FORE);`, which is a
        // single `move.w #imm,d0`. It must carry the same two numbers, and the
        // pair the leaked header would give must appear nowhere at all.
        let packed =
            u16::from(AMIGA_DEFAULT_BACKGROUND) << 8 | u16::from(AMIGA_DEFAULT_FOREGROUND);
        let mut needle = vec![0x30u8, 0x3C];
        needle.extend(packed.to_be_bytes());
        assert_eq!(
            find_all(&terp, &needle).len(),
            1,
            "{who}: `move.w #${packed:04X},d0` — set_color's returned default pair",
        );
        assert!(
            find_all(&terp, &[0x0B, 0x09]).is_empty(),
            "{who}: (11 << 8) | 9 — the leaked yzip.h's pair — occurs nowhere in the program",
        );
    }
}
