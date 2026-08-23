//! The bitmap fonts Infocom shipped on the release floppies (SQ-0916).
//!
//! Four Amiga releases carry one, as an AmigaDOS disk font — a file that looks like
//! an executable and is not. `blorb::amiga_font` parses it; these cases pin it
//! against the real floppies, because a font parser that agrees only with its own
//! synthetic fixture agrees with nothing.
//!
//! Fixtures are gitignored, so every case skips vacuously when one is absent.

use std::path::PathBuf;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn font_on(disk: &str) -> Option<blorb::bitmap_font::BitmapFont> {
    let path = stories_dir().join(disk);
    if !path.is_file() {
        eprintln!("SKIP: gitignored floppy absent: {disk}");
        return None;
    }
    let files: Vec<(String, Vec<u8>)> = app::assets::files(&path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let n = f.name.clone();
            f.into_bytes().map(|b| (n, b))
        })
        .collect();
    let font = blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
    assert!(font.is_some(), "{disk} should carry a font");
    font
}

/// **Arthur ships a proportional 10×10 typeface**, and it is a text font — letters,
/// not the box-drawing set.
///
/// The proportional flag is the load-bearing fact: it means Arthur's Amiga text was
/// never monospaced, so nobody should expect a column-for-column match against an
/// Amiga screenshot, and it is why `Glyph::width` exists separately from the font's
/// nominal width.
#[test]
fn arthur_carries_a_proportional_text_font() {
    let Some(font) = font_on("Arthur - The Quest for Excalibur.adf") else { return };
    assert_eq!((font.width, font.height), (10, 10), "nominal cell");
    assert_eq!(font.baseline, 8);
    assert!(font.proportional, "FPF_PROPORTIONAL is set");
    assert_eq!(font.lo, 32, "starts at the space");
    assert_eq!(font.glyphs.len(), 127 - 32 + 1, "covers 32..=127");

    // A text font, not font 3: 'A' is a letter and the space is blank.
    let a = font.glyph(b'A').expect("'A'");
    assert_eq!(a.rows.len(), 10);
    assert_ne!(a.rows.iter().fold(0, |x, r| x | r), 0, "'A' is drawn");
    assert!(font.glyph(b' ').expect("space").rows.iter().all(|&r| r == 0), "the space is blank");

    // Proportional in fact and not just in flag: the widths really differ.
    let widths: std::collections::BTreeSet<u8> =
        (32u8..=126).filter_map(|c| font.glyph(c)).map(|g| g.width).collect();
    assert!(widths.len() > 3, "a proportional font has several widths, saw {widths:?}");
    assert!(widths.iter().all(|&w| w <= 8), "no glyph is wider than a byte: {widths:?}");
    assert!(font.glyph(b'i').unwrap().width < font.glyph(b'm').unwrap().width, "'i' is narrower than 'm'");

    // Descenders reach the last two rows, which is why an 8-row master would clip.
    for ch in *b"gpqyj" {
        let g = font.glyph(ch).expect("descender");
        assert_ne!(g.rows[9], 0, "{} descends to the last row", char::from(ch));
    }
}

/// **Journey's font is the font-3 set, and it is byte-identical to Beyond Zork's.**
///
/// Same file under two names — `Char.data` on the v6 disk, `Graphic.Data` on the v5
/// one. That is what makes font 3 reachable from the v6 raster path, and it is the
/// reason `bitfont`'s font-3 entries are not dead code (see that module's header).
#[test]
fn journey_carries_the_font_three_set_and_not_a_typeface() {
    let Some(font) = font_on("Journey - The Quest Begins.adf") else { return };
    assert_eq!((font.width, font.height), (8, 8));
    assert!(!font.proportional, "the font-3 set is fixed-pitch");
    assert_eq!(font.lo, 32);
    // Every glyph is the full cell width — that is what fixed-pitch means here.
    assert!(
        (32u8..=126).filter_map(|c| font.glyph(c)).all(|g| g.width == 8),
        "font 3 glyphs are all 8 wide",
    );
    // NOT a typeface: code 65 is a solid block in font 3, not the letter A. Pinned
    // as "the inked rows are all the same run", which is true of a block and false
    // of every letterform — an 'A' has an apex, a crossbar and two legs.
    let a = font.glyph(b'A').expect("code 65");
    let inked: Vec<u8> = a.rows.iter().copied().filter(|&r| r != 0).collect();
    assert!(inked.len() >= 4, "code 65 is drawn");
    assert!(
        inked.iter().all(|&r| r == inked[0]),
        "font-3 code 65 is a solid block, not a letter: {:02X?}",
        a.rows,
    );
    // The solid block at code 54 is the whole cell, which no text font has.
    assert_eq!(font.glyph(54).expect("code 54").rows, vec![0xFF; 8], "code 54 is the full cell");
}

/// Shogun and Zork Zero ship no font at all — they take the system topaz.
///
/// Asserted so the loader's "no font on this medium" path stays exercised against a
/// real disk rather than only against an empty `Vec`.
#[test]
fn shogun_and_zork_zero_carry_no_font() {
    for disk in ["James Clavell's Shogun.adf", "Zork Zero - The Revenge of Megaboz.adf"] {
        let path = stories_dir().join(disk);
        if !path.is_file() {
            eprintln!("SKIP: gitignored floppy absent: {disk}");
            continue;
        }
        let files: Vec<(String, Vec<u8>)> = app::assets::files(&path)
            .into_iter()
            .filter(|f| f.is_on_medium())
            .filter_map(|f| {
                let n = f.name.clone();
                f.into_bytes().map(|b| (n, b))
            })
            .collect();
        let got =
            blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())));
        assert!(got.is_none(), "{disk} carries no font, but one parsed");
    }
}

/// **Macintosh resource forks are readable, and the v6 releases keep a font there**
/// (SQ-0911).
///
/// `hfs.rs` read only data forks until this quest, on the correct grounds that
/// Infocom keeps its story and its artwork in them. It keeps its FONTS in the
/// resource fork, which is what reopened that decision — see SQ-0916 for what the
/// fonts turned out to be good for, which is less than hoped.
///
/// Zork Zero's Macintosh floppy is the fixture because it is a plain HFS volume: the
/// forks on the *Lost Treasures* CD are ISO9660 associated files, which
/// `iso9660.rs` skips by design and which this does not exercise.
#[test]
fn the_macintosh_floppy_carries_a_font_in_its_resource_fork() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent");
        return;
    }
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("readable")).expect("mounts");

    // The application is ALL resource fork — zero data bytes — which is the case
    // that would read as an empty file if only data forks were reachable.
    let app_entry = hfs
        .files()
        .iter()
        .find(|e| e.file_type == *b"APPL")
        .expect("the volume carries an application");
    assert_eq!(app_entry.size, 0, "a Macintosh application is all resource fork");
    assert!(app_entry.resource_size > 30_000, "and the fork is substantial");

    let fork = hfs.read_resource(app_entry).expect("the resource fork reads");
    assert_eq!(fork.len(), app_entry.resource_size, "the whole fork, not a prefix");
    let rf = blorb::resource_fork::ResourceFork::parse(&fork).expect("parses as a resource fork");
    assert!(rf.types.len() > 5, "an application carries many types: {}", rf.types.len());
    assert!(!rf.of_type(b"CODE").is_empty(), "an application has CODE");

    // The payload this quest was reopened for.
    let font = blorb::mac_font::from_fork(&rf).expect("a FONT resource with a bitmap");
    assert_eq!((font.width, font.height), (7, 15), "the body face");
    assert_eq!(font.baseline, 12, "ascent");
    assert!(font.glyphs.len() > 200, "covers the Mac roman range");
    let a = font.glyph(b'A').expect("'A'");
    assert_eq!(a.rows.len(), 15);
    assert_ne!(a.rows.iter().fold(0, |x, r| x | r), 0, "'A' is drawn");
    assert!(font.glyph(b' ').expect("space").rows.iter().all(|&r| r == 0), "the space is blank");

    // The family-name record (id ≡ 0 mod 128) carries no bitmap and must be refused
    // rather than parsed into a zero-sized font.
    let family = rf.of_type(b"FONT").iter().find(|r| r.id % 128 == 0);
    if let Some(r) = family {
        assert!(blorb::mac_font::parse(&r.data).is_none(), "FONT {} is a family record", r.id);
    }
}

/// The Macintosh font is **fixed-pitch across printable ASCII, at 7** — and that is
/// still one pixel narrower than lanthorn's cell (SQ-0916).
///
/// This case exists because the first reading of this font was wrong twice. It was
/// called proportional (true only if you count the accented high range, which no
/// game prints), and it was previewed without its left side bearings, which flushed
/// every glyph left and made evenly-advanced text look raggedly letter-spaced. With
/// the bearings applied the spacing is uniform — it is simply one pixel loose per
/// character, because the v6 cell is 8.
///
/// So the obstacle to drawing with these glyphs is not the font. It is that using
/// them properly needs the font's METRICS as well, which is a change to what
/// `$26`/`$27` tell the game and moves every window the game lays out. Pinned so
/// that stays measured rather than remembered.
#[test]
fn the_macintosh_font_is_fixed_pitch_but_narrower_than_our_cell() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent");
        return;
    }
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("readable")).expect("mounts");
    let font = hfs
        .files()
        .iter()
        .filter(|e| e.resource_size > 0)
        .filter_map(|e| hfs.read_resource(e))
        .filter_map(|f| blorb::resource_fork::ResourceFork::parse(&f))
        .find_map(|rf| blorb::mac_font::from_fork(&rf))
        .expect("a Macintosh FONT");

    let widths: std::collections::BTreeSet<u8> = (33u8..=126)
        .filter_map(|c| font.glyph(c))
        .filter(|g| g.rows.iter().any(|&r| r != 0))
        .map(|g| g.width)
        .collect();
    assert_eq!(
        widths,
        std::collections::BTreeSet::from([7]),
        "every printable character advances by the same 7 — this is a fixed-pitch face \
         over the range a game prints, whatever the accented high range does",
    );
    assert!(
        widths.iter().all(|&w| w < 8),
        "and it is NARROWER than lanthorn's 8px cell, so drawing with it at that cell \
         costs a pixel of tracking on every character: {widths:?}",
    );

    // The bearings really are applied: a narrow glyph is not flush against column 0.
    let l = font.glyph(b'l').expect("'l'");
    assert!(
        l.rows.iter().filter(|&&r| r != 0).all(|r| r & 0x80 == 0),
        "'l' should sit inside its advance, not flush left: {:02X?}",
        l.rows,
    );
}

/// **The Macintosh face actually reaches the renderer** (SQ-1011).
///
/// `native_disk_font`'s other cases prove the PARSER reads `FONT` 524 correctly.
/// This one proves the app-side resolution does too — which is a different claim,
/// and the one that was false when this was written: the feature shipped inert and
/// a before/after render diff came back byte-identical on all four frames.
///
/// Each step is asserted separately so a failure says WHICH link broke rather than
/// just that the chain did.
#[test]
fn the_macintosh_face_resolves_for_the_renderer() {
    let path = stories_dir().join("Zork Zero Disk.image");
    if !path.is_file() {
        eprintln!("SKIP: gitignored Macintosh medium absent");
        return;
    }
    // 1. the volume mounts and carries the face
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("readable")).expect("mounts");
    let face = blorb::mac_font::from_volume(&hfs).expect("the volume carries FONT 524");
    assert_eq!((face.width, face.height), (7, 15), "the face is the Macintosh cell");
    // NOT `!face.proportional` — that flag counts the accented high range and is
    // `true` for this face. What matters is the printable set (SQ-0916, and the
    // case above measures it as exactly {7}).
    let printable: std::collections::BTreeSet<u8> =
        (b'!'..=b'~').filter_map(|c| face.glyph(c)).map(|g| g.width).collect();
    assert_eq!(printable, [7].into(), "every printable character advances by the cell");

    // 2. the profile the medium resolves to declares that same cell
    let (profile, source) = app::interpreter::InterpreterProfile::resolve_with_source(
        &path, None, None, None,
    );
    assert_eq!(profile, app::interpreter::InterpreterProfile::Macintosh, "the medium names the Mac");
    assert_eq!(profile.v6_font_cell(), (7, 15), "which declares 7x15");
    assert_eq!(
        source,
        app::interpreter::ProfileSource::Medium,
        "and it came from the MEDIUM — `native_font::resolve` gates on this",
    );

    // 3. so the resolver hands the renderer a face
    let resolved = app::native_font::resolve(&path, profile, source);
    assert!(resolved.is_some(), "native_font::resolve must find it — the renderer takes this or nothing");
}
