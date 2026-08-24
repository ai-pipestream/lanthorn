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

    // Proportional in fact and not just in flag: the advances really differ.
    let widths: std::collections::BTreeSet<u8> =
        (32u8..=126).filter_map(|c| font.glyph(c)).map(|g| g.width).collect();
    assert!(widths.len() > 3, "a proportional font has several widths, saw {widths:?}");
    assert!(font.glyph(b'i').unwrap().width < font.glyph(b'm').unwrap().width, "'i' is narrower than 'm'");
    // The INK fits a byte — that is `Glyph::rows`' limit, and it is a different
    // fact from the advance, which reaches the nominal 10 on the widest codes.
    assert!(
        widths.iter().any(|&w| w > 8),
        "an advance is not a strike width and is not capped at 8: {widths:?}",
    );

    // Descenders reach the last two rows, which is why an 8-row master would clip.
    for ch in *b"gpqyj" {
        let g = font.glyph(ch).expect("descender");
        assert_ne!(g.rows[9], 0, "{} descends to the last row", char::from(ch));
    }
}

/// **The advance comes from `tf_CharSpace`/`tf_CharKern`, not from the strike**
/// (SQ-1009).
///
/// The parser read `tf_CharLoc`'s bit width as the pen advance for as long as this
/// face has been parsed, which is wrong by a pixel or two on every glyph and by
/// EVERYTHING on the space: a space has no ink, so its strike is zero bits wide and
/// text drawn at that advance has no gaps between its words at all. The face was
/// only ever laid out in a fixed cell, so nothing had noticed.
///
/// The numbers below are also the independent confirmation that
/// `machine-screenshots/amiga-arthur*.png` are **1:1 native 320x200** captures and
/// not halved 640-wide ones, which three notes on SQ-1009 could not settle from the
/// pixels alone. They agree at 1:1 and are out by a factor of two at 2:1.
#[test]
fn the_amiga_advance_is_the_pen_and_not_the_strike() {
    let Some(font) = font_on("Arthur - The Quest for Excalibur.adf") else { return };

    // A blank glyph that still moves the pen — the whole defect in one assertion.
    let space = font.glyph(b' ').expect("space");
    assert!(space.rows.iter().all(|&r| r == 0), "the space has no ink");
    assert_eq!(space.width, 3, "and still advances three pixels");

    for (ch, advance) in [(b'i', 3), (b'm', 8), (b'W', 8), (b'T', 6), (b'h', 5), (b'e', 5)] {
        assert_eq!(
            font.glyph(ch).expect("glyph").width,
            advance,
            "{}'s advance",
            char::from(ch),
        );
    }

    // `machine-screenshots/amiga-arthur-hint.png`: the InvisiClues highlight box
    // around `THE CHURCHYARD` measures 83 px wide, which this run of text fills to
    // within the box's own padding. At 2:1 the same run would need 160.
    let run: u32 = b"THE CHURCHYARD".iter().map(|&c| u32::from(font.glyph(c).unwrap().width)).sum();
    assert_eq!(run, 80, "the pen crosses a 320-wide screen, not a 640-wide one");

    // `machine-screenshots/info.txt` measures Arthur's prose at ~4.5 px/char.
    let prose = b"The wind moans in the churchyard.";
    let px: u32 = prose.iter().map(|&c| u32::from(font.glyph(c).unwrap().width)).sum();
    let per = f64::from(px) / prose.len() as f64;
    assert!((4.0..5.5).contains(&per), "prose averages ~4.5 px/char, measured {per:.2}");
}

/// **Three runs off a real Amiga screen, predicted to the pixel** (SQ-1009).
///
/// `machine-screenshots/amiga-arthur-text.png` is the opening prose at 2x, and the
/// ink span of a line of it is a function of every advance in that line — so this
/// is a per-glyph check wearing the shape of one number, and the strongest evidence
/// in the repo that the corrected reading is right rather than merely plausible.
/// The strike-width reading missed all three.
///
/// The measured spans are the white-pixel extents of those rows in the capture,
/// halved: 628, 634 and 204 device pixels at 2x.
#[test]
fn the_advance_table_predicts_a_real_amiga_frame_to_the_pixel() {
    let Some(font) = font_on("Arthur - The Quest for Excalibur.adf") else { return };

    // First inked column of the first glyph to last inked column of the last, which
    // is what a screen shows — the trailing side bearing of the final glyph leaves
    // no mark, so the pen total is a pixel wider than the ink on all three.
    let ink = |s: &str| -> u32 {
        let (mut pen, mut first, mut last) = (0u32, None, 0u32);
        for c in s.bytes() {
            let g = font.glyph(c).expect("glyph");
            for r in &g.rows {
                for b in 0..8u32 {
                    if r & (0x80 >> b) != 0 {
                        first.get_or_insert(pen + b);
                        last = last.max(pen + b);
                    }
                }
            }
            pen += u32::from(g.width);
        }
        last - first.expect("some ink") + 1
    };

    for (run, measured_at_2x) in [
        ("WHOSO PULLETH OUT THIS SWORD OF THIS STONE, IS RIGHTWISE KING", 628),
        ("You are shivering in the cold night air of an English churchyard, unsure", 634),
        ("BORN OF ALL ENGLAND.", 204),
    ] {
        assert_eq!(
            ink(run) * 2,
            measured_at_2x,
            "amiga-arthur-text.png measures {measured_at_2x} px of ink for {run:?}",
        );
    }
}

/// The fixed-pitch faces carry NEITHER array, and there the advance is `tf_XSize`.
///
/// Journey's and Beyond Zork's font-3 sets store `tf_CharSpace` and `tf_CharKern` as
/// null pointers. Reading a null as an offset of zero would give every glyph the
/// first entry of whatever happens to sit at the start of the hunk.
#[test]
fn a_fixed_pitch_face_advances_by_its_nominal_cell() {
    let Some(font) = font_on("Journey - The Quest Begins.adf") else { return };
    assert_eq!(font.width, 8, "the nominal cell");
    let widths: std::collections::BTreeSet<u8> =
        (32u8..=255).filter_map(|c| font.glyph(c)).map(|g| g.width).collect();
    assert_eq!(widths, [8].into_iter().collect(), "every code advances one cell");
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
    assert_eq!(profile.v6_font_cell(), zvm::interpreter::MACINTOSH_V6_CELL, "which declares 7x15");
    assert_eq!(
        source,
        app::interpreter::ProfileSource::Medium,
        "and it came from the MEDIUM — `native_font::resolve` gates on this",
    );

    // 3. so the resolver hands the renderer a face. `disks: None` — this is the
    // RELEASE rung's claim, and a case here must not depend on what the person
    // running it keeps in `~/.lanthorn/` (SQ-1037).
    let resolved = app::native_font::resolve(&app::native_font::FaceRequest {
        story_path: &path,
        entry: None,
        profile,
        source,
        art_scale: None,
        disks: None,
    });
    assert!(
        resolved.body().is_some(),
        "native_font::resolve must find it — the renderer takes this or nothing",
    );
    assert!(
        resolved.fixed().is_some(),
        "and it is the machine's FIXED-PITCH face, which is the role it fills once a \
         System disk can supply the body one (SQ-1036)",
    );
}

/// A compilation volume pairs the face with ONE story, not with the platter
/// (SQ-1018).
///
/// The Masterpieces CD is the case the volume-wide lookup could not answer: 38
/// applications, and the first one enumerated is *A Mind Forever Voyaging*, a
/// Version 4 title shipping no `FONT` at all. `mac_font::from_volume` took that
/// first `APPL` and stopped, so every graphical game on the disc resolved to no
/// face — and then drew its 7x15 Macintosh cell with the 8-wide fallback, which
/// is legible enough to look like a rendering opinion rather than a missing
/// resource. That silence is why this went unnoticed while SQ-0876 caught the
/// identical defect in the artwork half, where the failure is visible: every
/// game on this same disc was drawing Zork Zero's plates.
///
/// The three v6 titles all carry `FONT` 524 in their own folder, so the fix is
/// the pairing rather than a fallback.
#[test]
fn a_compilation_pairs_the_face_with_the_story_beside_it() {
    let path = stories_dir().join("InfocomMasterpieces.img");
    if !path.is_file() {
        eprintln!("SKIP: gitignored compilation volume absent");
        return;
    }
    let hfs = blorb::hfs::Hfs::mount(std::fs::read(&path).expect("readable")).expect("mounts");

    // NON-VACUITY: this disc must actually BE the trap, or the case below proves
    // nothing. Many applications, and the first one carries no font.
    let appls: Vec<_> = hfs.files().iter().filter(|e| e.file_type == *b"APPL").collect();
    assert!(appls.len() > 1, "a compilation, not a single-game floppy: {} APPL", appls.len());
    let first = hfs
        .read_resource(appls[0])
        .and_then(|f| blorb::resource_fork::ResourceFork::parse(&f))
        .and_then(|rf| blorb::mac_font::from_fork(&rf));
    assert!(
        first.is_none(),
        "the FIRST application on the platter ships no face — that is the whole defect",
    );

    // The story this volume opens by its own tiebreak, which is what a launch
    // with no picker row behind it gets.
    let (opened, _) = hfs.story().expect("the disc carries a game");
    assert_eq!(opened, "InfocomMasterpieces/ZORK ZERO/STORY.DATA", "Zork Zero wins the tiebreak");

    // Paired with its own folder, the face is there.
    let face = blorb::mac_font::from_volume_beside(&hfs, &opened).expect("Zork Zero's own FONT 524");
    assert_eq!((face.width, face.height), (7, 15), "the Macintosh cell");

    // And it is per-game, not per-disc: Arthur's folder answers for Arthur.
    let arthur = blorb::mac_font::from_volume_beside(
        &hfs,
        "InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA",
    )
    .expect("Arthur's own FONT 524");
    assert_eq!((arthur.width, arthur.height), (7, 15), "the same cell, its own resource");

    // End to end, which is the claim that matters: the renderer gets a face.
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
    assert_eq!(profile, app::interpreter::InterpreterProfile::Macintosh, "the medium names the Mac");
    assert_eq!(source, app::interpreter::ProfileSource::Medium, "off the volume");
    assert!(
        app::native_font::resolve(&app::native_font::FaceRequest {
            story_path: &path,
            entry: None,
            profile,
            source,
            art_scale: None,
            disks: None,
        })
        .body()
        .is_some(),
        "the renderer takes this or nothing — `None` here is the reported defect",
    );
}

/// What the browser's info panel is told about a release's typefaces (SQ-1018).
///
/// Arthur's Macintosh pressing carries two — the 7x15 body face and the 7x12
/// ALT face `mac/xzip.lst` selects as `ZALT` — and only the first is drawn. The
/// panel showing both, with one marked, is what turns SQ-1017 from an invisible
/// omission into something a person can see; it is also what would have shown
/// SQ-1018 as "present, not used" instead of a report about crowded text.
///
/// Paired by ENTRY, so this is Arthur's answer and not the platter's: the disc's
/// own tiebreak opens Zork Zero, which ships no ALT face at all.
#[test]
fn the_panel_sees_both_macintosh_faces_and_only_one_in_use() {
    let path = stories_dir().join("InfocomMasterpieces.img");
    if !path.is_file() {
        eprintln!("SKIP: gitignored compilation volume absent");
        return;
    }
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(&path, None, None, None);
    let panel = |entry: &'static str| {
        app::native_font::detected(&app::native_font::FaceRequest {
            story_path: &path,
            entry: Some(entry),
            profile,
            source,
            art_scale: None,
            disks: None,
        })
    };
    let faces = panel("InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA");
    let named = |n: &str| {
        faces.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{n} listed: {faces:?}"))
    };

    let body = named("FONT 524");
    assert_eq!((body.width, body.height), (7, 15), "the body face is the Macintosh cell");
    assert!(body.used, "and it is the one the renderer takes");

    let alt = named("FONT 1033");
    assert_eq!((alt.width, alt.height), (7, 12), "the ALT face has its own cell");
    assert!(!alt.used, "carried and not drawn — SQ-1017");

    // Zork Zero ships only the body face, which is why its InvisiClues banner
    // cannot be explained by a second one (SQ-0934).
    let zz = panel("InfocomMasterpieces/ZORK ZERO/STORY.DATA");
    assert_eq!(zz.len(), 1, "Zork Zero carries one face, not two: {zz:?}");
    assert_eq!(zz[0].name, "FONT 524");
    assert!(zz[0].used);
}

// ── the TEXT scale is not the ART scale (SQ-1039) ────────────────────────────

/// A synthetic proportional face `height` rows tall, with a real advance per glyph.
///
/// Synthetic on purpose: no medium in `stories/` carries a `Metric` face for the
/// Macintosh — Geneva lives in the System file that shipped with the machine and
/// with no game (SQ-1036) — so the press that exposes SQ-1039 cannot be reached
/// through a fixture at all. The metrics are Geneva 12's as
/// `unit_tests/macfont.hfs` reports them: fifteen rows, advances spanning 3 to 11.
fn proportional_face(height: u8) -> blorb::bitmap_font::BitmapFont {
    let glyph = |w: u8| blorb::bitmap_font::Glyph {
        width: w,
        // Solid ink, so `measure_proportional` counts the glyph — it excludes blank
        // ones, since an undefined character carries a zero advance in both formats.
        rows: vec![0xFF; usize::from(height)],
    };
    let widths: Vec<u8> = (b'\x20'..=b'\x7e').map(|c| 3 + (c % 9)).collect();
    blorb::bitmap_font::BitmapFont {
        width: 11,
        height,
        baseline: height - 3,
        bold_smear: 0,
        proportional: true,
        lo: b' ',
        glyphs: widths.into_iter().map(glyph).collect(),
    }
}

/// **A `Metric` face's declared line takes the TEXT scale, and the Macintosh's is
/// 1:1 however dense its artwork is** (SQ-1039).
///
/// `art_scale` is the ARCHIVE's — how many native pixels one PICTURE pixel becomes
/// — and the Version 6 cell is the MACHINE's. Scaling a typeface by the art scale
/// conflates them, and on one press that is wrong rather than merely imprecise:
///
/// | press | picture space | `art_scale` | one art px | one text px |
/// |---|---|---|---|---|
/// | Macintosh colour | `CPic.data` 320x200 | (2, 2) → 640x400 | **2 native** | 1 native |
/// | Macintosh B/W | `Pic.data` 480x300 | (1, 1) | 1 native | 1 native |
/// | Amiga | 320x200 | (2, 2) → 640x400 | 2 native | 1 native |
///
/// The Amiga's face is authored in the picture space, so doubling the art doubles
/// the face with it and `height * 2` is right — measured, not assumed: Arthur's
/// advance table averages 5.21 face px per character while
/// `machine-screenshots/amiga-arthur-text.png` measures 4.70 ART px, which agree at
/// 1:1 and are out by a factor of two at 2:1. The Macintosh paints text at one
/// native pixel per face pixel while doubling `CPic.data` around it, so Geneva 12's
/// fifteen rows were being declared as thirty.
///
/// **The monochrome press cannot falsify this**, which is why the colour row is here:
/// `Pic.data` is (1, 1) and 15 x 1 is 15 under either rule.
#[test]
fn a_metric_faces_declared_line_takes_the_text_scale_not_the_archives() {
    use app::interpreter::InterpreterProfile as P;
    use app::native_font::{declared_cell, fit, FaceFit};

    let face = proportional_face(15);
    // A RELEASE face — the space that follows a story's own medium (SQ-1053), which
    // is the picture space on the Amiga and native pixels on the Macintosh.
    let released = |p: P, art: (u32, u32)| {
        app::native_font::FaceSet::release(face.clone(), p, Some(art))
    };
    // Non-vacuity: the whole quest is about `Metric` faces, and a face that fell to
    // `Cell` — or was declined — would pass every assertion below for free.
    assert_eq!(
        fit(&face, P::Macintosh, (1, 1)),
        Some(FaceFit::Metric),
        "admitted on the Macintosh",
    );
    assert_eq!(fit(&face, P::Amiga, (2, 2)), Some(FaceFit::Metric), "admitted on the Amiga");

    assert_eq!(
        declared_cell(P::Macintosh, &released(P::Macintosh, (2, 2)), (2, 2)).h,
        15,
        "the Macintosh COLOUR press declares the face's own fifteen rows, not thirty",
    );
    assert_eq!(
        declared_cell(P::Macintosh, &released(P::Macintosh, (1, 1)), (1, 1)).h,
        15,
        "the monochrome press agrees, as it does under either rule",
    );
    assert_eq!(
        declared_cell(P::Amiga, &released(P::Amiga, (2, 2)), (2, 2)).h,
        30,
        "the Amiga's RELEASE face IS in the picture space, so a doubled press doubles it",
    );
    // The WIDTH never follows the face on either machine: a proportional face has no
    // single advance, and this repo does not guess a declared metric.
    assert_eq!(declared_cell(P::Macintosh, &released(P::Macintosh, (2, 2)), (2, 2)).w, 7);
    assert_eq!(declared_cell(P::Amiga, &released(P::Amiga, (2, 2)), (2, 2)).w, 8);
}

/// **The PEN takes the same scale the cell does** (SQ-1039).
///
/// `TextFace` holds one scale and all three consumers read it — the declared cell,
/// the advance table `zvm` measures and wraps with, and `render::bitfont`'s
/// per-glyph blit. So a machine whose text is native-pixel must advance and draw at
/// 1:1 while its artwork stays doubled, and asserting the stored scale is asserting
/// all three at their source.
#[test]
fn the_pen_and_the_blit_take_the_text_scale_too() {
    use app::interpreter::InterpreterProfile as P;
    use app::native_font::TextFace;

    let face = proportional_face(15);
    let mac = TextFace::new(
        P::Macintosh,
        app::native_font::FaceSet::release(face.clone(), P::Macintosh, Some((2, 2))),
        Some((2, 2)),
    );
    let amiga = TextFace::new(
        P::Amiga,
        app::native_font::FaceSet::release(face, P::Amiga, Some((2, 2))),
        Some((2, 2)),
    );

    assert_eq!(mac.scale(), (1, 1), "the Macintosh draws one native pixel per face pixel");
    assert_eq!(amiga.scale(), (2, 2), "the Amiga doubles its face with its artwork");
    assert!(mac.proportional() && amiga.proportional(), "non-vacuity: both pens are the face's");

    // The advance is the glyph's own width on the Macintosh, and twice it on the
    // Amiga — the same ratio the cell moved by, which is the point.
    for ch in ['i', 'm', 'W', ' '] {
        assert_eq!(
            amiga.advance(ch),
            2 * mac.advance(ch),
            "{ch:?}: the Amiga's pen is the Macintosh's doubled",
        );
        assert!(mac.advance(ch) > 0, "{ch:?}: non-vacuity, the face covers it");
    }
    assert_eq!(amiga.run_px("moonlight"), 2 * mac.run_px("moonlight"));
    assert_eq!(mac.line_px(), 15, "and the line is the face's own fifteen rows");
    assert_eq!(amiga.line_px(), 30);
}
