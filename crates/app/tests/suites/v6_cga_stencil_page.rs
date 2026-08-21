//! SQ-0956 — a two-colour stencil loses its ground when the machine pair is
//! licensed: Zork Zero's white page paints out its own CGA artwork on a DOS press.
//!
//! # As reported
//!
//! *DOS Zork Zero with the CGA rendition shows a white page bleeding into its
//! artwork* — and, in the same breath, the constraint that shapes the fix: **the
//! PC background SHOULD be white for Zork Zero normally, just not for CGA.** So
//! this is not "the IBM PC's page is wrong". It is "a two-colour rendition needs a
//! different ground from the one the story asked for".
//!
//! # The two machine screenshots that settle it
//!
//! `machine-screenshots/dos-zorkzero.png` — the Banquet Hall on the COLOUR
//! rendition, **25.7% `#FFFFFF`**. White, and the user confirms white is right
//! there: Zork Zero issues `set_colour(fg=2, bg=9)` on a window the size of the
//! screen and a full-colour plate has no quarrel with it.
//!
//! `machine-screenshots/dos-zorkzero-cga.png` — the same room, same release, a DOS
//! emulator in CGA mode running `zork0.cg1`. Censused whole at 507x317:
//!
//! | share | value     | what                                   |
//! |-------|-----------|----------------------------------------|
//! | 48.3% | `#000000` | the page                               |
//! |  8.8% | `#A0A0A0` | the ink                                |
//! |   —   | 161 hues  | a grey ramp from video scaling, no second colour |
//!
//! Row parity was checked before the census, because an interlaced capture
//! censuses backwards (SQ-0933): even rows 39,252 black / 7,135 grey, odd rows
//! 38,391 / 6,968 — they agree, so the whole-frame number is the honest one.
//!
//! **That is the story's own pair INVERTED.** Zork Zero asks for black ink on a
//! white page; the CGA screen is light ink on a black page. So the page is not the
//! story's to set here, which is what `PictSource::declines_game_colours` exists
//! to say.
//!
//! **What is NOT claimed is that the display has no colour choice at all.** The
//! user went back to the emulator: Zork Zero's in-game `color` command on CGA
//! offers a **swap** of the two states, black and light grey, and nothing else. A
//! two-colour display is two states and one bit of choice — it cannot name
//! arbitrary colours, which is the only thing declining §8.3's palette to the
//! story asserts. (The same visit also found that the swapped, light-ground state
//! washes the plates out on the real machine exactly as it does for us: the art is
//! light line work authored for a black ground. The swap is SQ-0957's and is out
//! of scope here; nothing below implements or assumes it.)
//!
//! # Why the rule had stopped saying it
//!
//! SQ-0806's rule was `is_monochrome() && machine_pair.is_none()`, written when
//! `InterpreterProfile::IbmPc` stated no defaults at all. SQ-0928 gave it blue
//! under white, and `ProfileSource::Medium` licenses a machine's colours — so on
//! the one kind of launch CGA art actually comes from, a real DOS press, the guard
//! read the licence as the fact it was standing down for and the rule went quiet.
//!
//! The discriminator is now the machine's PAGE rather than whether it named one,
//! and it is **one channel**: `two_colour_colours` against `default_colours`.
//!
//! | machine   | its screen | its two-colour display | declines |
//! |-----------|------------|------------------------|----------|
//! | IBM PC    | (6, 9)     | **(2, 9)**             | yes      |
//! | Macintosh | (9, 2)     | (9, 2)                 | no       |
//!
//! The Macintosh is not exempted anywhere; it states its two-colour page once
//! instead of twice, so the rule passes over it by construction. `v6_macintosh_profile`
//! is where that half is pinned.
//!
//! # Specimens
//!
//! One release, one press, one machine — and the archive is the only thing that
//! moves, which is as clean a control as this corpus offers.
//!
//! | fixture                          | release      | archive served | two-colour |
//! |----------------------------------|--------------|----------------|------------|
//! | DOS 360K **Disk 1**              | r393/s890714 | `.CG1` (CGA)   | yes        |
//! | DOS 360K **Disk 3**              | r393/s890714 | `.EG1` (EGA)   | no         |
//! | DOS 720K **Disk 2**              | r393/s890714 | `.CG1` (CGA)   | yes        |
//! | DOS 720K **Disk 1**              | r393/s890714 | `.MG1` (MCGA)  | no         |
//!
//! Each volume is opened directly and `crate::assets::volumes` mounts the rest of
//! the set around it, so which plate a launch gets is decided by the disk the
//! player put in — no `--pictures` anywhere here, because the reported launch had
//! none. **Turn count: zero.** Every frame below is the boot banner, flushed
//! through the real elems pipeline; Zork Zero paints its ornate border and its
//! window-0 page before it asks for anything, so the bleed is on screen at the
//! first prompt and driving keys would only add ways for the frame to differ.
//!
//! Both `honor_game_colours` modes are pinned throughout, per the project's colour
//! convention — and here the `true` half is the load-bearing one, since it is the
//! mode the defect was reported in and the mode lanthorn ships.
//!
//! The DOS press is gitignored commercial media, so every case skips vacuously
//! without it and [`the_press_was_actually_read`] is what stops the file quietly
//! passing on a machine that has none of it.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::{InterpreterProfile, ProfileSource};
use app::session::GameSession;
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global, and the harness below sets it
/// (SQ-0904/SQ-0905).
static PALETTE: &std::sync::Mutex<()> = &app::V6_PALETTE_LOCK;

const RELEASE: u16 = 393;
const SERIAL: &[u8] = b"890714";

/// The volume serving the CGA plate on the 360K press — the reported launch.
const CGA_DISK: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 1) [!].ima";
/// The same press's EGA volume: the control that must keep its colours.
const EGA_DISK: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (360K) (Disk 3) [!].ima";
/// The 720K press, where the two plates sit the other way round.
const CGA_DISK_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 2) [!].ima";
const MCGA_DISK_720: &str = "Zork Zero - The Revenge of Megaboz (1989) (r393, Serial 890714) (Infocom, Inc.) (720K) (Disk 1) [!].ima";

/// Every specimen, with what the press serves off it.
const PRESS: &[(&str, bool)] =
    &[(CGA_DISK, true), (EGA_DISK, false), (CGA_DISK_720, true), (MCGA_DISK_720, false)];

/// A pane roomy enough for Zork Zero's chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn present(file: &str) -> bool {
    stories_dir().join(file).exists()
}

/// What `startup.rs` decided about this launch, before any rendering.
struct Decision {
    profile: InterpreterProfile,
    source: ProfileSource,
    monochrome: bool,
    /// `Config::machine_default_colours` — the machine's own screen.
    machine_pair: Option<(u8, u8)>,
    /// `Config::machine_two_colour_colours` — the same machine, two-colour display.
    two_colour_pair: Option<(u8, u8)>,
    declines: bool,
}

/// `startup.rs`'s colour sequence for one volume, run through the real
/// [`app::config::Config`] so the licence gate is the shipped one and not a
/// re-implementation of it.
fn decide(file: &str) -> Option<Decision> {
    let path = stories_dir().join(file);
    if !path.exists() {
        eprintln!("SKIP: gitignored DOS press missing at {}", path.display());
        return None;
    }
    let dir = std::env::temp_dir().join(format!("lanthorn-sq956-decide-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    // No `--pictures`: the reported launch named no archive, and the medium is
    // what serves the plate.
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    let picts = PictSource::resolve_with_override(&path, over, None);
    let cfg = app::config::Config {
        interpreter_profile: profile,
        interpreter_source: source,
        ..Default::default()
    };
    let machine_pair = cfg.machine_default_colours();
    let two_colour_pair = cfg.machine_two_colour_colours();
    let d = Decision {
        profile,
        source,
        monochrome: picts.is_monochrome(),
        machine_pair,
        two_colour_pair,
        declines: picts.declines_game_colours(machine_pair, two_colour_pair),
    };
    let _ = std::fs::remove_dir_all(&dir);
    Some(d)
}

/// One booted-and-rendered frame, plus the honour answer that produced it.
struct Frame {
    buf: Buffer,
    viewport: Rect,
    honoured: bool,
    /// The story window's own background — Zork Zero's `set_colour` page, as the
    /// §8.3.1 number it declared and as the colour the render resolves it to
    /// (through the IBM PC's palette and the player's theme, never a literal).
    story_bg: zvm::screen::ZColour,
    story_page: Option<ratatui::style::Color>,
    /// The model and the state the frame was drawn from, so the other surfaces
    /// that fill a transparent hole can be asked the same question.
    model: app::engine::ScreenModel,
    state: AppState,
}

/// Boot one volume the way `startup.rs` boots — the medium picks the profile and
/// the palette, the archive has its say about colours, the four screen-size links
/// resolve in order — and render one hybrid frame of the boot banner.
///
/// `force_honour` is the caller overruling the archive, which is how the
/// pre-SQ-0956 screen is reproduced for comparison: `Some(true)` honours the
/// story's colours whatever the plate says.
fn frame(file: &str, force_honour: Option<bool>) -> Option<Frame> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored DOS press missing at {}", path.display());
            return None;
        }
    };
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "this press carries r{RELEASE}");
    assert_eq!(&bytes[0x12..0x18], SERIAL);

    let dir = std::env::temp_dir().join(format!("lanthorn-sq956-frame-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let over = PictureOverride::resolve_with_session(&path, &dir, None);
    let named_art_std_window = over.std_window();
    let (profile, source) = InterpreterProfile::resolve_with_source(&path, None, over.flavour(), None);
    zvm::screen::set_palette(zvm::interpreter::palette_for(
        profile.row_number(),
        bytes.first().copied(),
    ));
    let mut picts = PictSource::resolve_with_override(&path, over, None);
    let picture_dims = picts.all_pict_dims();
    let std_window = picts
        .std_window()
        .or(named_art_std_window)
        .or_else(|| picts.native_std_window())
        .or_else(|| profile.std_window());

    let cfg = app::config::Config {
        interpreter_profile: profile,
        interpreter_source: source,
        ..Default::default()
    };
    let honoured = force_honour.unwrap_or(
        !picts.declines_game_colours(cfg.machine_default_colours(), cfg.machine_two_colour_colours()),
    );
    let mut session = GameSession::new_with_art_scale(
        bytes,
        honoured,
        false,
        cfg.advertised_interpreter_number(),
        false,
        picture_dims,
        std_window,
        picts.art_scale(),
        honoured.then(|| cfg.machine_default_colours()).flatten(),
        None,
        None,
    )
    .expect("Zork Zero boots off the DOS press");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = std::fs::remove_dir_all(&dir);

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);

    let model = session.screen();
    let (story_bg, story_page) = {
        let app::engine::WinNode::Layered(items) = &model.root else {
            panic!("v6 builds a Layered root")
        };
        let story = app::render::v6_layout::classify_windows(items).story;
        let (_, b) = app::render::v6_layout::story_pair_packed(story);
        // `story_bg_rgba` is the very function `render::screen` floods the pane
        // with, so the colour compared below is the one the frame was painted in.
        let page = app::render::v6_layout::story_bg_rgba(story, &state.colors)
            .map(|p| ratatui::style::Color::Rgb(p[0], p[1], p[2]));
        (app::state::unpack_zcolour(b), page)
    };
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    let viewport = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    Some(Frame { buf, viewport, honoured, story_bg, story_page, model, state })
}

/// How many cells of the story viewport are read on `page`.
fn cells_on(f: &Frame, page: ratatui::style::Color) -> usize {
    let vp = f.viewport;
    (vp.y..vp.bottom())
        .flat_map(|y| (vp.x..vp.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| f.buf.cell((x, y)).is_some_and(|c| c.bg == page))
        .count()
}

/// The plate's own region: every cell of the pane the story viewport does not
/// cover — Zork Zero's ornate border, its flanks and the banner above them.
fn art_region(f: &Frame) -> impl Iterator<Item = (u16, u16)> + '_ {
    let vp = f.viewport;
    (PANE.y..PANE.bottom())
        .flat_map(|y| (PANE.x..PANE.right()).map(move |x| (x, y)))
        .filter(move |&(x, y)| {
            !(x >= vp.x && x < vp.right() && y >= vp.y && y < vp.bottom())
        })
}

/// **How much of the line work survives**, as two numbers and the ground they
/// were counted against.
///
/// A halfblocks picker writes each vertical pixel pair as a cell's foreground over
/// its background, so a cell of the plate carries the two colours that landed
/// there — which makes the drawn buffer an honest oracle for detail (the same
/// reason `v6_float_machine_page` reaches for it).
///
/// **The ground is read off the frame itself**, as the commonest background in the
/// plate's own region, rather than being passed in: the point is how much of the
/// art can be told apart from whatever it ended up sitting on, and comparing two
/// frames against one fixed colour would score the frame whose ground is that
/// colour unfairly low for that reason alone. Returns `(distinct colours,
/// distinguishable cells, ground)`.
struct ArtDetail {
    hues: usize,
    lit: usize,
    ground: ratatui::style::Color,
}

fn art_detail(f: &Frame) -> ArtDetail {
    let mut grounds: std::collections::BTreeMap<String, (usize, ratatui::style::Color)> =
        Default::default();
    for (x, y) in art_region(f) {
        if let Some(c) = f.buf.cell((x, y)) {
            let e = grounds.entry(format!("{:?}", c.bg)).or_insert((0, c.bg));
            e.0 += 1;
        }
    }
    let ground = grounds.values().max_by_key(|(n, _)| *n).map(|(_, c)| *c).expect("a drawn pane");

    let mut hues = std::collections::BTreeSet::new();
    let mut lit = 0usize;
    for (x, y) in art_region(f) {
        let Some(c) = f.buf.cell((x, y)) else { continue };
        hues.insert(format!("{:?}", c.fg));
        hues.insert(format!("{:?}", c.bg));
        // A cell shows something only if some part of it is not the ground: an
        // all-ground cell is a hole, whatever glyph is nominally in it.
        let shows = |col: ratatui::style::Color| col != ground;
        if shows(c.bg) || (shows(c.fg) && c.symbol() != " ") {
            lit += 1;
        }
    }
    ArtDetail { hues: hues.len(), lit, ground }
}


// ── The premise ──────────────────────────────────────────────────────────────

/// **Non-vacuity.** Every case here skips without the gitignored press, so
/// something has to fail when the whole suite skips for a reason other than the
/// fixtures being absent — and something has to check that the press really does
/// serve the plate each specimen claims.
#[test]
fn the_press_was_actually_read() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let any = PRESS.iter().any(|(f, _)| present(f));
    let mut seen = 0usize;
    for (file, two_colour) in PRESS {
        let Some(d) = decide(file) else { continue };
        seen += 1;
        assert_eq!(d.profile, InterpreterProfile::IbmPc, "{file}: a DOS press is an IBM PC");
        assert_eq!(d.source, ProfileSource::Medium, "{file}: the medium named the machine");
        assert!(d.machine_pair.is_some(), "{file}: and a medium licenses that machine's colours");
        assert_eq!(
            d.monochrome, *two_colour,
            "{file}: the plate this volume serves — the archive is the only thing that moves \
             across this table",
        );
    }
    assert!(!any || seen > 0, "the press is on disk but not one volume was read");
}

// ── The rule ─────────────────────────────────────────────────────────────────

/// **One channel, and it is the whole discriminator** (SQ-0956).
///
/// Asserted on the machine table rather than on a launch, because this is the
/// fact the rule reads and everything else here follows from it. The ink does not
/// move: white 9 both times, which is the `#A0A0A0` the CGA capture measures and
/// the same value `dos-hitchhiker.png` gives for its ink on the same emulator
/// family.
#[test]
fn the_ibm_pcs_two_colour_display_moves_one_channel() {
    let pc = InterpreterProfile::IbmPc;
    let (page, ink) = pc.default_colours().expect("SQ-0928: the IBM PC states its pair");
    let (two_page, two_ink) = pc.two_colour_colours().expect("and states its two-colour page");
    assert_eq!((page, ink), (6, 9), "the machine's screen: blue under white");
    assert_eq!((two_page, two_ink), (2, 9), "its CGA plate: BLACK under the same white");
    assert_eq!(ink, two_ink, "one channel — the ink is the card's, unmoved");
    assert_ne!(page, two_page, "…and the page is not");
}

/// **THE DEFECT, at the decision that causes it.** The CGA volume of a real DOS
/// press declines the story's colours; the EGA volume beside it does not.
///
/// FALSIFIED by restoring the old guard (`machine_pair.is_none()`): the CGA rows
/// then report `declines = false`, because `ProfileSource::Medium` licenses the
/// IBM PC's pair and the guard reads that licence as a reason to stand down. That
/// is the reported launch exactly — no `--pictures`, just the disk in the drive.
#[test]
fn a_cga_volume_declines_the_storys_colours_and_an_ega_volume_does_not() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let any = PRESS.iter().any(|(f, _)| present(f));
    let mut seen = 0usize;
    for (file, two_colour) in PRESS {
        let Some(d) = decide(file) else { continue };
        seen += 1;
        assert_eq!(d.machine_pair, Some((6, 9)), "{file}: the IBM PC's own screen");
        assert_eq!(d.two_colour_pair, Some((2, 9)), "{file}: and its two-colour display");
        assert_eq!(
            d.declines, *two_colour,
            "{file}: a stencil declines the story's pair, a sixteen-colour plate has no reason to",
        );
    }
    assert!(!any || seen > 0, "the press is on disk but not one volume was read");
}

// ── The screen ───────────────────────────────────────────────────────────────

/// **THE DELIVERABLE, as reported:** *a white page bleeding into the artwork*, on
/// DOS Zork Zero with the CGA rendition.
///
/// Zork Zero issues `set_colour(fg=2, bg=9)` on a window the size of the screen
/// for every video card alike — it cannot see which plate was loaded — so with the
/// story's colours honoured that page floods the story viewport and the two-colour
/// border art is read on it. On the colour renditions that is right, and
/// `machine-screenshots/dos-zorkzero.png` shows it. On the CGA plate the machine
/// shows the opposite polarity entirely.
///
/// **The RELATION is asserted, never an RGB.** The page goes through the IBM PC's
/// palette and the player's theme, so a literal would be pinning the resolver
/// rather than the rule. What is pinned is that the ground the CGA frame is read
/// on is not the ground the story asked for — while the same frame with the story
/// honoured (the pre-fix screen, reproduced here on purpose) is covered in it.
///
/// **And then what the user actually reported, which a colour equality cannot
/// see.** On the real machine the black background punches through the plate in
/// its transparent areas; in lanthorn those areas stayed white, the light line
/// work sat on white, *and a lot of detail simply disappeared*. A ground
/// assertion passes the moment one pixel is right — so the second half counts how
/// much of the plate can be told apart from whatever it ended up sitting on, each
/// frame against its own ground. Measured on the boot frame, both presses:
///
/// | the story's page | frame's ground | distinct colours | cells lit |
/// |------------------|----------------|------------------|-----------|
/// | honoured         | `#FFFFFF`      | 251              | 1,115     |
/// | declined         | `#000000`      | 256              | **1,672** |
///
/// FALSIFIED by restoring the old guard: the `declined` column becomes the
/// `honoured` one, the story viewport comes back flooded with white and the plate
/// loses those 557 cells again, which is the report verbatim. The `honoured`
/// column is also the non-vacuity guard — if the story ever stops setting that
/// page, this test proves nothing and says so.
#[test]
fn the_storys_white_page_does_not_reach_a_cga_frame() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let any = present(CGA_DISK) || present(CGA_DISK_720);
    let mut seen = 0usize;
    for file in [CGA_DISK, CGA_DISK_720] {
        let Some(forced) = frame(file, Some(true)) else { continue };
        let Some(shipped) = frame(file, None) else { continue };
        seen += 1;
        assert!(forced.honoured, "{file}: the pre-fix frame honours the story's colours");
        assert_eq!(
            forced.story_bg,
            zvm::screen::ZColour::Standard(9),
            "{file}: premise — Zork Zero really does ask for §8.3.1's white page",
        );
        let white = forced.story_page.expect("an honoured page resolves to a colour");
        let flooded = cells_on(&forced, white);
        assert!(
            flooded > 0,
            "{file}: the symptom must reproduce with colours honoured, else this proves nothing",
        );

        // The detail first, because it is what was reported and what a colour
        // equality cannot see — and because reverting the fix must fail HERE,
        // with the line work gone, rather than on a ground that merely reads
        // differently.
        let before = art_detail(&forced);
        let after = art_detail(&shipped);
        eprintln!(
            "{file}: honoured {flooded} cells on the story's page {white:?}; art detail — {} \
             cells lit of {} colours on ground {:?} honoured, {} of {} on ground {:?} declined",
            before.lit, before.hues, before.ground, after.lit, after.hues, after.ground,
        );
        assert!(
            after.lit > before.lit && after.hues >= before.hues,
            "{file}: the plate's line work must SURVIVE — {} distinguishable cells of {} colours \
             when the story's page is honoured, {} of {} when it is not",
            before.lit,
            before.hues,
            after.lit,
            after.hues,
        );

        // …and the ground itself, which is what makes the detail above possible.
        assert!(!shipped.honoured, "{file}: the shipped frame declines them — SQ-0806's rule");
        assert_eq!(before.ground, white, "{file}: honoured, the plate sits on the story's page");
        assert_ne!(after.ground, white, "{file}: declined, it does not");
        assert_eq!(
            cells_on(&shipped, white),
            0,
            "{file}: not one cell of the story's page on a two-colour frame — {flooded} of them \
             before the rule fired again",
        );
    }
    assert!(!any || seen > 0, "a CGA volume is on disk but no frame was rendered");
}

/// **Every surface that fills a transparent hole takes the same page.**
///
/// `Picture::rgba_with` drops a two-colour plate's clear index to alpha 0, and
/// three different places resolve that hole against "the page of the moment":
/// `v6_layout::flatten_onto_page` for the raster composite,
/// `fill_story_page_clear`/`fill_window_pages` for the hybrid ring, and
/// `inline_image::float_page` for a story float. Fixing the page fixes all three
/// only for as long as none of them keeps a copy — so this asks each of them
/// directly rather than trusting that they still share the gate.
///
/// The raster composite is the sharpest of the three, because it flattens its
/// WHOLE canvas opaque before shipping: every hole in the plate becomes a real
/// pixel of the page. It is measured in PIXELS, and with the same detail oracle
/// as the ring — **not** by counting the story's white, which cannot tell the
/// bleed apart from the plate's own paint. `CGA_PALETTE`'s set bit is pure white,
/// so a `.CG1`'s line work IS white: measured on this frame, 221,263 white pixels
/// with the story's page honoured and 45,688 with it declined, of which the
/// second number is entirely the artwork. That is exactly the trap the user's
/// report describes from the other side — light line work on a white ground — and
/// it is why "how much can be told apart from the ground" is the honest question.
///
/// Measured on the boot frame, 360K disk 1 and 720K disk 2 alike:
///
/// | the story's page | composite's ground | distinct colours | pixels lit |
/// |------------------|--------------------|------------------|------------|
/// | honoured         | `#FFFFFF`          | **2**            | 34,737     |
/// | declined         | `#000000`          | **3**            | **61,341** |
///
/// Two colours in a two-colour plate is the collapse itself: with the white page
/// underneath, the plate's own white IS the ground and only its black strokes are
/// left to see. Declined, the ground is a third value and 1.77x as much of the
/// artwork can be told apart from it.
#[test]
fn the_raster_composite_and_the_floats_take_the_same_page_as_the_ring() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let any = present(CGA_DISK) || present(CGA_DISK_720);
    let mut seen = 0usize;
    for file in [CGA_DISK, CGA_DISK_720] {
        let Some(forced) = frame(file, Some(true)) else { continue };
        let Some(shipped) = frame(file, None) else { continue };
        seen += 1;
        let white = forced.story_page.expect("an honoured page resolves to a colour");
        let ratatui::style::Color::Rgb(r, g, b) = white else { panic!("{file}: an RGB page") };
        let white_px = image::Rgba([r, g, b, 255]);

        let mut raster = Vec::new();
        for (what, f, want_white) in [("honoured", &forced, true), ("declined", &shipped, false)] {
            // The raster composite, built exactly as `render::screen` builds it.
            let app::engine::WinNode::Layered(items) = &f.model.root else {
                panic!("v6 builds a Layered root")
            };
            let native = app::render::v6_layout::native_extent(items);
            let layout = app::render::v6_layout::classify_windows(items);
            let (canvas, _) =
                app::render::screen::build_v6_raster_canvas(&layout, native, &f.state);
            let mut tally: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
            for p in canvas.pixels() {
                *tally.entry(p.0).or_default() += 1;
            }
            let ground = *tally.iter().max_by_key(|(_, n)| **n).expect("a built canvas").0;
            let lit: usize = tally.iter().filter(|(c, _)| **c != ground).map(|(_, n)| *n).sum();
            let flooded = tally.get(&white_px.0).copied().unwrap_or(0);
            eprintln!(
                "{file} ({what}): raster ground {ground:?}, {lit} px lit of {} colours, {flooded} \
                 px of the story's white",
                tally.len(),
            );
            assert_eq!(
                ground == white_px.0,
                want_white,
                "{file} ({what}): the composite's own ground is the page of the moment",
            );
            raster.push(lit);

            // …and the ground a story float is flattened onto — layered by
            // `inline_image::float_page` over the very same story page.
            let float = app::render::inline_image::float_page(&f.state);
            assert_eq!(
                float == Some(white_px),
                want_white,
                "{file} ({what}): a float's ground must be the same page — got {float:?}",
            );
        }
        assert!(
            raster[1] > raster[0],
            "{file}: more of the plate must be distinguishable once the story's page is \
             declined — {} px lit honoured, {} px declined",
            raster[0],
            raster[1],
        );
    }
    assert!(!any || seen > 0, "a CGA volume is on disk but no frame was rendered");
}

/// **And the colour renditions of the same release are untouched** — which is the
/// user's own constraint: white is right for Zork Zero on a PC, just not for CGA.
///
/// Same press, same release, same machine; only the plate differs. The EGA and
/// MCGA volumes must still honour the story's page, and their frames must still be
/// read on it.
#[test]
fn the_colour_renditions_keep_the_white_page_they_asked_for() {
    let _g = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let any = present(EGA_DISK) || present(MCGA_DISK_720);
    let mut seen = 0usize;
    for file in [EGA_DISK, MCGA_DISK_720] {
        let Some(f) = frame(file, None) else { continue };
        seen += 1;
        assert!(f.honoured, "{file}: a sixteen-colour plate has colours to give");
        assert_eq!(
            f.story_bg,
            zvm::screen::ZColour::Standard(9),
            "{file}: the story asks for the same white it asks for on every card",
        );
        let white = f.story_page.expect("an honoured page resolves to a colour");
        assert!(
            cells_on(&f, white) > 0,
            "{file}: and it reaches the screen, exactly as `dos-zorkzero.png` shows it",
        );
    }
    assert!(!any || seen > 0, "a colour volume is on disk but no frame was rendered");
}
