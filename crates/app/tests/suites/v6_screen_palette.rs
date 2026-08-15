//! SQ-0887: on a machine with ONE screen palette, a scene's colours repaint the
//! art already on the screen — and on a machine with many, they must not.
//!
//! # The report
//!
//! Shogun's ornate side panels are blue-and-white in the storm on deck and
//! red-on-cream below decks on a real Amiga. babelmap drew them gold-on-dark in
//! every scene. And — the half that makes this a machine question rather than a
//! bug with one right answer — the DOS press leaves them one colour throughout,
//! which is *also* correct. One story, one border, two machines, two behaviours.
//!
//! # Why it never fired
//!
//! `PictSource::image` opened with a fast path for a source that declares no
//! adaptive pictures, and Shogun's Amiga `Pic.data` declares none: all 48 of its
//! pictures carry a palette of their own. So the Current Palette was never
//! established, `palette_gen` never moved, and SQ-0567's replay — which exists
//! precisely to recolour what is already drawn — never ran at all.
//!
//! # What decides it
//!
//! The hardware, carried as `MachineProfile::one_screen_palette`. The Amiga has
//! one set of colour registers; the MCGA's DAC has 256 entries and Infocom used
//! them, which is why Arthur's map screen holds three palettes at once (SQ-0881).
//! An archive cannot answer this — both presses give every picture its own
//! table — so the profile does, and the app hands it down at boot.
//!
//! The fixture is gitignored, so every case skips vacuously without it.

use std::path::PathBuf;

use app::graphics::PictSource;

/// Shogun's Amiga floppy — release 295, serial 890321.
const SHOGUN_ADF: &str = "James Clavell's Shogun.adf";

/// The picture numbers this suite reasons about, measured off that disk by
/// logging every `image` call through a real boot and reading back each one's
/// own palette:
///
/// | pict | own palette, entry 2 | what it is |
/// |---|---|---|
/// | 3 | `#CCAA66` gold | the ornate border |
/// | 7 | `#99BBDD` blue | the storm on deck |
/// | 8 | `#CC0000` red | below decks |
const BORDER: u32 = 3;
const STORM: u32 = 7;
const BELOW_DECKS: u32 = 8;

fn shogun_archive() -> Option<PictSource> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories").join(SHOGUN_ADF);
    let raw = std::fs::read(&p).ok()?;
    let disk = blorb::medium::MountedDisk::mount(raw).ok()?;
    Some(PictSource::from_native(disk.pictures()?.pictures))
}

/// A digest of a decoded Pict's whole pixel buffer.
///
/// The WHOLE buffer, not a sample: the border's own centre is transparent — it
/// is a frame around the screen, so its middle is where the scene goes — and
/// index 0 is black in every one of these palettes, so a point sample compares
/// two colours that agree no matter which table drew them. That cost an
/// afternoon once; take the whole picture.
fn digest(img: Option<std::sync::Arc<image::DynamicImage>>) -> Option<u64> {
    let rgba = img?.to_rgba8();
    Some(rgba.pixels().enumerate().fold(0u64, |h, (i, p)| {
        h.wrapping_mul(31).wrapping_add(u64::from(p.0[0]) << 16)
            ^ (u64::from(p.0[1]) << 8 | u64::from(p.0[2])).wrapping_add(i as u64)
    }))
}

/// **The premise**: this archive declares nothing adaptive, and every picture
/// carries its own colours. Without that the rest of the suite is about nothing.
#[test]
fn the_amiga_archive_declares_no_adaptive_pictures_and_gives_each_its_own() {
    let Some(mut src) = shogun_archive() else {
        eprintln!("SKIP: gitignored medium missing: {SHOGUN_ADF}");
        return;
    };
    assert!(!src.is_adaptive(BORDER), "the border is not declared adaptive");
    assert!(!src.is_adaptive(STORM), "nor is the storm");
    // Three pictures, three different palettes — so "which one is the screen's"
    // is a real question rather than a distinction without a difference.
    // The flag has to be on to observe this at all: with it off, `image` takes
    // the no-adaptive fast path and never records a palette — which IS the bug.
    src.set_screen_palette(true);
    let mut seen = Vec::new();
    for n in [BORDER, STORM, BELOW_DECKS] {
        let _ = src.image(n);
        seen.push(src.current_palette().map(<[u8]>::to_vec));
    }
    assert!(seen.iter().all(Option::is_some), "each picture states a palette of its own");
    seen.dedup();
    assert_eq!(seen.len(), 3, "and the three differ");
}

/// **The fix.** With the machine flag on, the border replays through whichever
/// scene palette is live; with it off, it keeps its own.
///
/// Both halves are asserted because the flag has to be a *switch*: the Amiga
/// needs the first behaviour and the IBM PC needs the second, and a change that
/// only delivered one of them would be SQ-0881 or this bug, in turn.
///
/// Falsifiable: drop `self.screen_palette ||` from
/// `PictSource::image_under_current_palette` and the border comes back gold in
/// both cases; make it unconditional and the DOS press starts recolouring.
#[test]
fn one_screen_palette_repaints_the_border_and_many_palettes_leave_it_alone() {
    let Some(mut src) = shogun_archive() else {
        eprintln!("SKIP: gitignored medium missing: {SHOGUN_ADF}");
        return;
    };
    let own = digest(src.image(BORDER)).expect("the border decodes");

    // A ONE-PALETTE machine (the Amiga): the scene below decks is drawn, and the
    // border already on the screen is shown through the palette it loaded.
    src.set_screen_palette(true);
    let _ = src.image(BELOW_DECKS);
    let under_reds = digest(src.image_under_current_palette(BORDER)).expect("replays");
    assert_ne!(under_reds, own, "the border follows the scene on a one-palette machine");

    // …and a different scene gives it a different colour again, which is what
    // rules out "it just decodes to something else once".
    let _ = src.image(STORM);
    let under_blues = digest(src.image_under_current_palette(BORDER)).expect("replays");
    assert_ne!(under_blues, under_reds, "each scene, not one substitution");

    // A MANY-PALETTE machine (the IBM PC, whose DAC holds 256 entries): the same
    // archive, the same scenes, and the border keeps the colours it carries.
    src.set_screen_palette(false);
    let _ = src.image(BELOW_DECKS);
    assert_eq!(
        digest(src.image_under_current_palette(BORDER)),
        Some(own),
        "with many palettes the border is untouched — Shogun's DOS press, and SQ-0881's rule"
    );
}

/// The flag is the MACHINE's, and exactly one machine claims it today.
///
/// Pinned so that turning another on is a deliberate act with a measurement
/// behind it, per the machine table's sourced-or-declined standard — the
/// Macintosh and the Atari ST plausibly qualify and neither has been measured.
#[test]
fn only_the_amiga_claims_one_screen_palette() {
    let claiming: Vec<&str> = zvm::interpreter::MACHINES
        .iter()
        .filter(|m| m.one_screen_palette)
        .map(|m| m.name)
        .collect();
    assert_eq!(claiming, vec!["Amiga"], "one machine, on one measurement");
    let ibm = zvm::interpreter::machine(zvm::interpreter::IBM_PC_INTERPRETER_NUMBER)
        .expect("the IBM PC is modelled");
    assert!(!ibm.one_screen_palette, "the MCGA's DAC has 256 entries (SQ-0881)");
}
