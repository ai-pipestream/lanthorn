//! Scout the typefaces a release's own medium carries, and why one is or is not
//! drawn (SQ-1018/SQ-1019).
//!
//! ```text
//! cargo run -p app --example font_scout -- [--entry <in-volume path>] <image>...
//! ```
//!
//! The browser's info panel answers "which faces does this release have". This
//! answers "why", which is a different question and the one a defect lives in: it
//! prints every link of the chain separately — the medium, the profile it names,
//! the cell that declares, the applications on the volume, every `FONT`/`NFNT`
//! resource **including the ones that do not parse**, and both the volume-wide
//! lookup and the paired one, side by side.
//!
//! That separation is the whole point. SQ-1018 was reported as crowded text and
//! was one line of this output: the Masterpieces CD's first application prints as
//! `A MIND FOREVER VOYAGING` with no `FONT` under it, and `from_volume` stopped
//! there. From the panel the game simply had no face, which is a symptom and not
//! a cause.
//!
//! `--entry` names one story inside a compilation, as `HfsEntry::path` spells it
//! (`InfocomMasterpieces/ARTHUR FOLDER/STORY.DATA`), and applies to every image
//! after it. Without one, the volume's own tiebreak chooses — which is exactly
//! what a launch with no picker row behind it gets, so the default is the case
//! worth checking first.
//!
//! Non-HFS media are not skipped: an AmigaDOS disk font is a file rather than a
//! resource, so the profile/detected/resolve block above still reports it and only
//! the volume dump below is Macintosh-specific.

fn main() {
    let mut entry: Option<String> = None;
    let mut glyphs: Vec<char> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--entry" {
            match args.next() {
                Some(e) => entry = Some(e),
                None => {
                    eprintln!("font_scout: --entry needs an in-volume path");
                    std::process::exit(2);
                }
            }
            continue;
        }
        if arg == "--glyph" {
            match args.next() {
                Some(g) => glyphs.extend(g.chars()),
                None => {
                    eprintln!("font_scout: --glyph needs one or more characters");
                    std::process::exit(2);
                }
            }
            continue;
        }
        scout(std::path::Path::new(&arg), entry.as_deref(), &glyphs);
    }
}

fn scout(path: &std::path::Path, entry: Option<&str>, glyphs: &[char]) {
    println!("\n=== {} ===", path.display());
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            println!("  unreadable: {e}");
            return;
        }
    };
    println!("  {} bytes, looks_like_hfs={}", bytes.len(), blorb::hfs::Hfs::looks_like_hfs(&bytes));

    // What the renderer is working from, in the order the answers are decided.
    let (profile, source) =
        app::interpreter::InterpreterProfile::resolve_with_source(path, None, None, None);
    println!("  profile={profile:?} source={source:?} cell={:?}", profile.v6_font_cell());
    if let Some(e) = entry {
        println!("  entry: {e}");
    }
    for f in app::native_font::detected(path, entry) {
        println!(
            "  detected: {:<10} {}x{}{}{}",
            f.name,
            f.width,
            f.height,
            if f.proportional { " proportional" } else { "" },
            if f.used { "  <- in use" } else { "" },
        );
    }
    println!(
        "  native_font::resolve -> {:?}",
        app::native_font::resolve(path, entry, profile, source).map(|f| (f.width, f.height)),
    );

    // Below here is the Macintosh volume itself: the layer the two lookups differ
    // on, and the only place a fork that will not parse is visible at all.
    let hfs = match blorb::hfs::Hfs::mount(bytes) {
        Ok(h) => h,
        Err(e) => {
            println!("  (not an HFS volume: {e:?})");
            return;
        }
    };
    println!("  volume: {:?}", hfs.volume_name());
    let appls: Vec<_> = hfs.files().iter().filter(|e| e.file_type == *b"APPL").collect();
    println!("  {} files, {} APPL:", hfs.files().len(), appls.len());
    for e in &appls {
        println!(
            "    APPL {:?} creator={:?} data={} rsrc={}",
            e.path(),
            String::from_utf8_lossy(&e.creator),
            e.size,
            e.resource_size,
        );
        let Some(fork) = hfs.read_resource(e) else {
            println!("      resource fork UNREADABLE");
            continue;
        };
        let Some(rf) = blorb::resource_fork::ResourceFork::parse(&fork) else {
            println!("      fork PARSE FAILED ({} bytes)", fork.len());
            continue;
        };
        let types: Vec<String> = rf
            .types
            .iter()
            .map(|(t, v)| format!("{}x{}", String::from_utf8_lossy(t), v.len()))
            .collect();
        println!("      types: {}", types.join(" "));
        for ty in [b"FONT", b"NFNT"] {
            for r in rf.of_type(ty) {
                // A failure is printed rather than filtered: id ≡ 0 (mod 128) is
                // the family-NAME record and carries no bitmap, so `FONT 512`
                // failing is correct and looks alarming until you know that.
                let what = match blorb::mac_font::parse(&r.data) {
                    Some(f) => format!(
                        "{}x{} baseline={} proportional={} lo={}",
                        f.width, f.height, f.baseline, f.proportional, f.lo,
                    ),
                    None => "no bitmap (family record, or not a font)".into(),
                };
                println!(
                    "      {} {:>5} {:>6}B -> {what}",
                    String::from_utf8_lossy(ty),
                    r.id,
                    r.data.len(),
                );
                // `--glyph A` is how you tell a TYPEFACE from the font-3 graphics
                // set, which is a different thing wearing the same shape: font 3's
                // code 65 is a solid block, and every letterform has an apex, a
                // crossbar and two legs. Reading the bitmap is the only way to
                // know, and a metrics line cannot say it.
                if let Some(f) = blorb::mac_font::parse(&r.data) {
                    for &c in glyphs {
                        print_glyph(&f, c);
                    }
                }
            }
        }
    }
    for e in hfs.files().iter().filter(|e| e.resource_size > 0 && e.file_type != *b"APPL") {
        println!(
            "    rsrc {:?} type={:?} rsrc={}",
            e.path(),
            String::from_utf8_lossy(&e.file_type),
            e.resource_size,
        );
    }

    // The two lookups, side by side — the disagreement IS the defect class.
    let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
    println!("  story opened: {opened:?}");
    let dims = |f: blorb::bitmap_font::BitmapFont| (f.width, f.height);
    println!("  from_volume (whole platter) -> {:?}", blorb::mac_font::from_volume(&hfs).map(dims));
    if let Some(p) = &opened {
        println!(
            "  from_volume_beside (this game) -> {:?}",
            blorb::mac_font::from_volume_beside(&hfs, p).map(dims),
        );
        let faces = blorb::mac_font::faces_beside(&hfs, p);
        println!("  faces_beside -> {:?}", faces.iter().map(|(id, f)| (*id, f.width, f.height)).collect::<Vec<_>>());
    }
}

/// One glyph as pixels, because whether a face is a typeface is a question about
/// its shapes and nothing else answers it.
fn print_glyph(font: &blorb::bitmap_font::BitmapFont, ch: char) {
    let Ok(code) = u8::try_from(ch as u32) else {
        println!("        {ch:?}: not a byte code");
        return;
    };
    let Some(g) = font.glyph(code) else {
        println!("        {ch:?} (0x{code:02X}): not in this font");
        return;
    };
    println!("        {ch:?} (0x{code:02X}) advance={} :", g.width);
    for (i, row) in g.rows.iter().enumerate() {
        let bits: String =
            (0..8).map(|b| if row & (0x80 >> b) != 0 { '#' } else { '.' }).collect();
        let baseline = if i + 1 == usize::from(font.baseline) { " <- baseline" } else { "" };
        println!("          {bits}{baseline}");
    }
}
