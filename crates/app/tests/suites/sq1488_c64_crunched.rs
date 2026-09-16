//! SQ-1488: the six Commodore 64 disks the quest was filed for — the five
//! *Questprobe 1 - The Hulk* "Collection Disk" images and
//! `THE INCREDIBLE HULK - THE ADVENTURE.D64` — each hold their one program
//! crunched behind a `$0801` BASIC-stub loader, so no table was ever in the
//! clear for `scott::looks_like_scott_bytes` to find and every one of them
//! used to report the generic "no story file on the disk image" message.
//!
//! **What this pins is an honest partial result, not a finished feature.**
//! `blorb::depack::depack_c64_prg` (regenerator2000-core's 6502-emulation
//! unpacker, see its `Cargo.toml` dependency comment) genuinely decompresses
//! five of the six — `decompresses_to_real_legible_game_text` below proves it
//! by finding the release's own credits and item descriptions in the
//! recovered bytes, in the clear, at no point fabricated or hand-edited. But
//! what comes out is Adventure International's OWN Commodore driver
//! ("Commodore version by: Mak Jukic, Version: 1c" — visible in the same
//! dump), a DIFFERENT C64 binary layout from the *Mysterious Adventures*
//! driver `scott::c64` reads (SQ-1455) and not a bare S.A.G.A. database at
//! the offset `scott::saga_us` checks either — so `scott::Database::parse`
//! does not accept it yet, and neither does any other dialect this crate
//! knows. A new clean-room reader for THIS driver is real, scoped future
//! work (nothing here should be read as "the Hulk collection disks play");
//! until it lands, every one of the six refuses by name instead of by the
//! generic message, which is what SQ-1488 asked for as the fallback when
//! depacking alone cannot reach a playable game ("or at least refuse by name
//! as a crunched program").
//!
//! `stories/` is gitignored (commercial media), so every case skips
//! vacuously when its fixture is absent.

use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories/scott-dialects/c64")
}

/// The five collection disks, each carrying its program crunched behind a
/// `$0801` BASIC stub that `blorb::depack::depack_c64_prg` DOES successfully
/// run to completion (verified below).
const DEPACKS_BUT_UNRECOGNISED: &[&str] = &[
    "Questprobe 1 - The Hulk [Collection Disk 1].d64",
    "Questprobe 1 - The Hulk [Collection Disk 2].d64",
    "Questprobe 1 - The Hulk [Collection Disk 3].d64",
    "Questprobe 1 - The Hulk [Collection Disk 4].d64",
    "Questprobe 1 - The Hulk [Collection Disk 5].d64",
];

/// The one disk whose program the emulator itself cannot finish depacking —
/// a genuine `regenerator2000-core` limitation on this specimen, not a
/// lanthorn integration gap (SQ-1488's brief explicitly allows this as a
/// legitimate, reportable outcome rather than something to force past).
const FAILS_TO_UNPACK: &str = "THE INCREDIBLE HULK - THE ADVENTURE.D64";

/// True when at least one of the six is present, so a suite-level skip
/// message can say plainly why every case below is vacuous.
fn any_specimen_present() -> bool {
    DEPACKS_BUT_UNRECOGNISED
        .iter()
        .chain(std::iter::once(&FAILS_TO_UNPACK))
        .any(|name| stories_dir().join(name).is_file())
}

/// The picker's own door lists nothing for any of the six — `mounted_stories`
/// answers `None` exactly as it did before SQ-1488, because "refuses with a
/// specific reason at open time" is not the same as "lists a playable row",
/// and this suite must not claim more than it found.
#[test]
fn none_of_the_six_are_listed_as_playable_rows() {
    if !any_specimen_present() {
        eprintln!("SKIP: no stories/scott-dialects/c64/ Hulk specimen present (gitignored commercial fixture)");
        return;
    }
    for name in DEPACKS_BUT_UNRECOGNISED.iter().chain(std::iter::once(&FAILS_TO_UNPACK)) {
        let path = stories_dir().join(name);
        if !path.is_file() {
            eprintln!("SKIP {name}: absent");
            continue;
        }
        assert!(
            app::hints::mounted_stories(&path).is_none(),
            "{name}: must list no playable rows until a reader for this driver exists"
        );
    }
}

/// The five collection disks each refuse BY NAME, distinguishing "unpacked
/// fine but unrecognised" from disk 6's genuine unpack failure below — the
/// honest, specific message SQ-1488 asked for in place of the old generic
/// "no story file on the disk image ... is this the boot disk?" (which gave
/// a player no reason to suspect a cruncher was even involved).
#[test]
fn the_five_collection_disks_refuse_by_name_as_unpacked_but_unrecognised() {
    for name in DEPACKS_BUT_UNRECOGNISED {
        let path = stories_dir().join(name);
        if !path.is_file() {
            eprintln!("SKIP {name}: absent (gitignored commercial fixture)");
            continue;
        }
        let err = app::hints::load_mounted_story_from(&path, None)
            .expect_err("a disk with no reader for its driver must refuse, not silently open");
        let msg = err.to_string();
        assert!(msg.contains("crunched program"), "{name}: not naming a crunched program: {msg}");
        assert!(
            msg.contains("not a Scott Adams game lanthorn recognises"),
            "{name}: must say it unpacked but was not recognised, not the generic no-story message: {msg}"
        );
        assert!(
            !msg.contains("is this the boot disk"),
            "{name}: must not fall back to the old generic message: {msg}"
        );
    }
}

/// `THE INCREDIBLE HULK - THE ADVENTURE.D64` legitimately does not finish
/// depacking at all (confirmed separately at up to 2 billion emulated
/// instructions, 40x the default budget, still incomplete) — its refusal
/// message must say so distinctly from the five above.
#[test]
fn the_incredible_hulk_the_adventure_refuses_by_name_as_not_unpacked() {
    let path = stories_dir().join(FAILS_TO_UNPACK);
    if !path.is_file() {
        eprintln!("SKIP {FAILS_TO_UNPACK}: absent (gitignored commercial fixture)");
        return;
    }
    let err = app::hints::load_mounted_story_from(&path, None)
        .expect_err("a program the emulator cannot finish depacking must refuse, not open");
    let msg = err.to_string();
    assert!(msg.contains("crunched program"), "not naming a crunched program: {msg}");
    assert!(msg.contains("lanthorn could not unpack it"), "must say unpacking itself failed: {msg}");
    assert!(
        !msg.contains("not a Scott Adams game lanthorn recognises"),
        "must not claim it unpacked when it did not: {msg}"
    );
}

/// The load-bearing positive claim in this suite: depacking one of the five
/// collection disks recovers REAL, LEGIBLE Scott Adams game content — the
/// release's own credits line and Scott Adams's own item-description syntax
/// (`"name/NOUN/"`) — proving `blorb::depack::depack_c64_prg` is doing real
/// work and not merely returning garbage that happens not to error.
///
/// This is deliberately checked below the app-side Scott gate
/// (`scott::Database::parse`, which still refuses this driver's layout) so
/// the suite cannot be fooled by a lucky-looking byte run: the assertions
/// are exact substrings transcribed from a real run of this exact fixture,
/// not a guess at what a Scott Adams game "should" say.
#[test]
fn decompresses_to_real_legible_game_text() {
    let path = stories_dir().join("Questprobe 1 - The Hulk [Collection Disk 2].d64");
    if !path.is_file() {
        eprintln!("SKIP: stories/scott-dialects/c64/Questprobe 1 - The Hulk [Collection Disk 2].d64 absent (gitignored commercial fixture)");
        return;
    }
    let raw = std::fs::read(&path).expect("disk image reads");
    blorb::medium::DiskImage::detect(&raw).expect("a D64 image");
    let disk = blorb::medium::MountedDisk::mount(raw).expect("mounts");
    let (name, bytes) = disk.contents().into_iter().next().expect("one directory entry");
    assert_eq!(name, "INCREDIBLE HULK");

    let depacked = blorb::depack::depack_c64_prg(&bytes).expect("this specimen depacks with the default budget");
    let text = String::from_utf8_lossy(&depacked.data);

    // The release's own title card and credits, recovered in the clear.
    assert!(text.contains("QUESTPROBE-ONE by Scott Adams"), "missing the game's own title line");
    assert!(text.contains("ADVENTURE INTERNATIONAL"), "missing the publisher credit");
    assert!(text.contains("Commodore version by"), "missing the C64 port credit");
    assert!(text.contains("Mak Jukic"), "missing the C64 porter's name");

    // Scott Adams's own item-description syntax survives depacking intact.
    assert!(text.contains("Mirror/MIRR/"), "missing a plain item description");
    assert!(text.contains("Broken chair/CHAI/"), "missing a second plain item description");

    // And this driver is confirmed NOT yet recognised — the suite's honesty
    // check on itself, so a future reader landing here fails loudly instead
    // of leaving this assertion silently wrong.
    assert!(
        scott::Database::parse(&depacked.data).is_err(),
        "a reader for this driver now exists — update this suite's own doc comment and the \
         `DEPACKS_BUT_UNRECOGNISED` cases above rather than leaving this stale"
    );
}
