//! A reproducible gallery of presentable frames (SQ-0942).
//!
//! WHAT THIS IS FOR. A project page needs pictures, and a terminal app is hard
//! to sell in stills. [`super::driver`] already boots the REAL binary under a
//! pty from a command line, and [`super::raster`] already draws the screen a
//! terminal would resolve out of the bytes it wrote back. What was missing was
//! a tool whose output is meant to be LOOKED at rather than measured, and whose
//! whole set regenerates from one file each release so the page cannot drift
//! from the build.
//!
//! WHY IT IS NOT THE ORACLE WITH A NICER FONT. `raster` is a geometry oracle and
//! its own docs are emphatic that it is not a screenshot. Handing THAT a real
//! typeface would be actively harmful: it would stop looking synthetic while
//! still not being anyone's terminal — no hinting, wrong face, wrong metrics —
//! and a picture that is 90% convincing invites exactly the judgement it cannot
//! support. So the font lives here, behind a flag the tests never pass, and
//! every frame this module writes carries a burnt-in label saying what it is.
//! [`label`] is not decoration; it is the reason a real face is allowed at all.
//!
//! THE RECIPE IS THE COMMITTED ARTEFACT. `examples/gallery.toml` is the input;
//! the PNGs are output and belong under `target/`. Nothing here records a
//! release number or a turn count by hand — the release and serial are read out
//! of the header of the bytes the medium actually mounted, and the turn count is
//! counted off the key spec, so neither can drift from the frame it describes.
//!
//! THE TWO TRAPS, ENCODED. A capture that does not negotiate kitty silently
//! measures the half-block backend, so [`Backend::Kitty`] shots FAIL rather than
//! quietly produce a picture of the wrong renderer. And the half-block picker
//! uses its own 10x20 font whatever the terminal reports, so [`Shot::cell`]
//! chooses the cell size from the backend rather than letting a manifest author
//! pick one that renders a geometry lanthorn was not using.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use image::{Rgba, RgbaImage};
use serde::Deserialize;

use super::driver::{self, Capture, Key, Spec};

/// The seed pinned into every shot's config unless the manifest overrides it.
///
/// Not cosmetic. fmvpoker and scopa deal randomly, and an unpinned gallery
/// regenerates differently every release for no reason at all: a first
/// comparison of two fmvpoker frames showed 37,097 differing pixels that were
/// entirely a different card deal, and none of them a render change.
pub const DEFAULT_SEED: u32 = 12345;

/// Which graphics backend a shot is taken through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    /// The default and the one the app ships for: art as kitty placements, text
    /// as terminal cells. A shot that fails to negotiate it is discarded.
    #[default]
    Kitty,
    /// The universal fallback: the same PIXEL path resolved into `▀`/`▄` cells.
    /// Worth a gallery row because it is what a reader on a terminal without
    /// graphics actually gets, and because it is the only v6 output an
    /// asciinema cast can carry (SQ-0943).
    Halfblocks,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Kitty => "kitty",
            Backend::Halfblocks => "halfblocks",
        }
    }
}

/// One frame in the gallery, exactly as `examples/gallery.toml` spells it.
///
/// Deliberately absent: the release, the serial and the turn count. All three
/// are DERIVED — the first two from the mounted story's header, the third from
/// [`Shot::keys`] — because a hand-written provenance line is a second copy of
/// the truth and this repo has been bitten by one before.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    /// Stable slug; becomes the PNG's filename, so it must survive being one.
    pub id: String,
    /// The game, for the caption.
    pub title: String,
    /// Which pressing — "Amiga floppy (.adf)", "Blorb", "Apple II 5.25-inch".
    /// A prose label for the reader; the machine-checkable half is the release
    /// read off the header at capture time.
    pub press: String,
    /// One or two sentences under the frame. The page is a gallery with
    /// captions, so this may be longer than the surrounding body text.
    pub caption: String,
    /// Path to the medium, relative to the repository root.
    pub media: String,
    /// The key spec, in [`Key::parse`]'s spelling: `cr,wait:900,text:look,cr`.
    pub keys: String,
    /// Terminal size in cells, `COLSxROWS`.
    pub size: String,
    /// Which backend to capture through.
    #[serde(default)]
    pub backend: Backend,
    /// Extra arguments passed through to lanthorn. The tool owns `--user-dir`
    /// and `--image-protocol`; naming either here is a manifest error.
    #[serde(default)]
    pub args: Vec<String>,
    /// The PRNG seed pinned for this shot.
    #[serde(default = "default_seed")]
    pub seed: u32,
    /// Keep the map pane. Off by default, so the story pane owns the frame.
    #[serde(default)]
    pub show_map: bool,
    /// Text that MUST be on the resolved screen, or the shot is discarded.
    ///
    /// The non-vacuity guard, and it earns its place. Pointed at a disk image
    /// holding several games, lanthorn opens a browser, and two blank keypresses
    /// picked *Ballyhoo* off a neighbouring floppy while every number in the
    /// record — release, serial, medium — went on describing the Zork Zero image
    /// the manifest named, because those are read from the file and not from the
    /// frame. Arthur's ProDOS press is the same failure more quietly: it renders
    /// identically at 6 and 40 keypresses because it never answers the restore
    /// question, and the still is of a boot prompt. One string off the screen
    /// catches both.
    #[serde(default)]
    pub expect: Vec<String>,
    /// The least number of cells a placement must actually cover.
    ///
    /// The guard for a frame with no text in it. Scopa and FMV Poker draw the
    /// whole screen as one composite — their buttons and their prompts are
    /// PICTURES — so a substring search over the cells finds nothing at all and
    /// would have to be waived. What those frames can assert instead is that the
    /// art landed, which is the same question SQ-0934 spent three rounds on when
    /// a cell harness reported "no art inside the viewport" and was believed.
    #[serde(default)]
    pub expect_art_cells: usize,
}

fn default_seed() -> u32 {
    DEFAULT_SEED
}

/// The whole manifest.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub shots: Vec<Shot>,
}

impl Manifest {
    /// Parse and validate. Every error a manifest author can plausibly make is
    /// caught here rather than three minutes into a capture run — and a test
    /// runs this over the committed file, so a broken manifest fails the gate
    /// instead of failing whoever next regenerates the page.
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let m: Manifest = toml::from_str(text).map_err(|e| format!("gallery manifest: {e}"))?;
        if m.shots.is_empty() {
            return Err("gallery manifest: no [[shots]] — an empty gallery is a mistake, not a choice".into());
        }
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for s in &m.shots {
            s.validate()?;
            if !seen.insert(s.id.as_str()) {
                return Err(format!("gallery manifest: duplicate shot id `{}` — ids are filenames", s.id));
            }
        }
        Ok(m)
    }

    /// The committed manifest's path: `crates/app/examples/gallery.toml`.
    pub fn default_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/gallery.toml")
    }

    /// Read and parse the committed manifest.
    pub fn load(path: &Path) -> Result<Manifest, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("gallery manifest: reading {}: {e}", path.display()))?;
        Manifest::parse(&text)
    }
}

/// The repository root, from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

impl Shot {
    fn validate(&self) -> Result<(), String> {
        let who = &self.id;
        if self.id.is_empty()
            || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "gallery manifest: shot id `{who}` must be lowercase ASCII, digits and dashes — it becomes a filename"
            ));
        }
        for (field, value) in [("title", &self.title), ("press", &self.press), ("caption", &self.caption), ("media", &self.media)] {
            if value.trim().is_empty() {
                return Err(format!("gallery manifest: `{who}` has an empty `{field}`"));
            }
        }
        self.size_cells().map(|_| ())?;
        self.keys().map(|_| ())?;
        // A half-block frame's art IS cells: `▀` with two colours, and not a
        // placement anywhere in the stream. `expect_art_cells` counts placements,
        // so on this backend it can only ever read zero and would fail every
        // shot that set it. The guard a half-block shot wants is `▀` in `expect`.
        if self.backend == Backend::Halfblocks && self.expect_art_cells > 0 {
            return Err(format!(
                "gallery manifest: `{who}` is a half-block shot with `expect_art_cells` — half-blocks \
                 emit no placements at all, so that count is always zero. Put `\u{2580}` in `expect` instead"
            ));
        }
        if self.expect.is_empty() && self.expect_art_cells == 0 {
            return Err(format!(
                "gallery manifest: `{who}` sets neither `expect` nor `expect_art_cells` — a shot with \
                 no guard is a shot that cannot tell its own frame from a browser or a boot prompt"
            ));
        }
        // The tool owns these, and a manifest that sets them either fights the
        // backend choice or writes the gallery into the player's real home.
        for owned in ["--image-protocol", "--user-dir", "--no-sound"] {
            if self.args.iter().any(|a| a == owned) {
                return Err(format!(
                    "gallery manifest: `{who}` passes `{owned}` — the gallery tool owns that argument \
                     (set `backend` instead of `--image-protocol`)"
                ));
            }
        }
        Ok(())
    }

    /// The terminal size in cells.
    pub fn size_cells(&self) -> Result<(u16, u16), String> {
        let (c, r) = self
            .size
            .split_once('x')
            .ok_or_else(|| format!("gallery manifest: `{}` has size `{}`, wanted COLSxROWS", self.id, self.size))?;
        let cols: u16 = c.trim().parse().map_err(|_| format!("gallery manifest: `{}`: bad column count in `{}`", self.id, self.size))?;
        let rows: u16 = r.trim().parse().map_err(|_| format!("gallery manifest: `{}`: bad row count in `{}`", self.id, self.size))?;
        if cols == 0 || rows == 0 {
            return Err(format!("gallery manifest: `{}` has a zero dimension in `{}`", self.id, self.size));
        }
        Ok((cols, rows))
    }

    /// The cell size in pixels this shot must be captured at — chosen by the
    /// BACKEND, never by the manifest.
    ///
    /// `Picker::halfblocks()` assumes a 10x20 cell whatever the terminal
    /// reported, so a half-block capture taken at any other size draws a
    /// geometry lanthorn was not using and every proportion in the picture is
    /// wrong. Kitty asks the terminal, so kitty gets the cell we answered
    /// `CSI 16 t` with — and what we answer is ours to choose well.
    ///
    /// BOTH CELLS ARE EXACTLY 1:2 (SQ-0963), and that is the point rather than a
    /// coincidence. A half-block sample is `cell_width` wide by `cell_height / 2`
    /// tall, so a square sample — equal resolution on both axes — wants a cell
    /// of exactly 1:2, and anything else samples the artwork finer across than
    /// down for no reason at all. The kitty cell was 8x18 and is now **8x16**:
    /// 1:2, the historical VGA text cell, the cell `app::render::bitfont`'s
    /// Uni-VGA master blits into 1:1 rather than resampling, and one of the ten
    /// sizes the gallery's own face lands a whole-numbered cell on (see
    /// [`FONT_CANDIDATES`]). `the_cell_is_square_for_half_block_samples` pins it.
    pub fn cell_px(&self) -> (u16, u16) {
        match self.backend {
            Backend::Kitty => (8, 16),
            Backend::Halfblocks => (10, 20),
        }
    }

    /// The story pane's CONTENT rect in cells — the box the v6 composite is
    /// magnified into — or `None` when this shot cannot answer.
    ///
    /// `compute_pane_layout` reserves one row for the help bar and nothing else
    /// while the command band and the inventory dock are closed (`layout.rs`),
    /// and `draw_framed` then insets the pane one cell on every side for its
    /// border. So a full-width story pane's content is `COLS - 2` by `ROWS - 3`.
    ///
    /// `None` for a map shot, deliberately. A split pane's width is a percentage
    /// of the frame resolved by ratatui, which is the app's arithmetic and not
    /// this file's to restate — and the one map shot in the manifest is a v3
    /// story with no pixel screen to magnify anyway.
    pub fn pane_content_cells(&self) -> Option<(u32, u32)> {
        if self.show_map {
            return None;
        }
        let (cols, rows) = self.size_cells().ok()?;
        Some((u32::from(cols).checked_sub(2)?, u32::from(rows).checked_sub(3)?))
    }

    /// How far the v6 composite is magnified in this shot: the aspect-preserving
    /// fit of `native` into the pane's device box, exactly as `uniform_scale`
    /// computes it (`v6_layout.rs`) — `min(box_w / native_w, box_h / native_h)`,
    /// unrounded and unclamped.
    ///
    /// WHY IT WANTS TO BE A WHOLE NUMBER (SQ-0963). At any other value every edge
    /// in the artwork is interpolated: the composite is resized once to
    /// `round(native * s)` and the bands are 1:1 crops out of that, so `s` is the
    /// only place softness can enter and a fractional `s` guarantees it. At an
    /// integer `s` one art pixel lands on a whole number of device pixels on both
    /// axes and the frame is exactly as crisp as the artwork is.
    ///
    /// This is a per-shot number and cannot be one constant: a Blorb press is
    /// 640x400, the standard Macintosh plate 480x304.
    pub fn magnification(&self, native: (u32, u32)) -> Option<f64> {
        let (cc, cr) = self.pane_content_cells()?;
        let (cw, ch) = self.cell_px();
        let (bw, bh) = (cc * u32::from(cw), cr * u32::from(ch));
        let (nw, nh) = (native.0.max(1), native.1.max(1));
        Some((f64::from(bw) / f64::from(nw)).min(f64::from(bh) / f64::from(nh)))
    }

    /// The scripted keys.
    pub fn keys(&self) -> Result<Vec<Key>, String> {
        self.keys
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| Key::parse(t).map_err(|e| format!("gallery manifest: `{}`: {e}", self.id)))
            .collect()
    }

    /// How many keypresses reached this frame — the turn count, counted off the
    /// spec rather than declared beside it.
    ///
    /// A frame is a fixture and this is half of its identity: Arthur's ProDOS
    /// press renders identically at 6 and 40 keypresses because it never answers
    /// the restore question, so "which frame is this" is unanswerable without it.
    pub fn turns(&self) -> usize {
        self.keys.split(',').filter(|t| {
            let t = t.trim();
            !t.is_empty() && !t.starts_with("wait:")
        }).count()
    }

    /// The arguments the tool adds on this shot's behalf.
    pub fn lanthorn_args(&self) -> Vec<String> {
        let mut v = self.args.clone();
        if self.backend == Backend::Halfblocks {
            v.push("--image-protocol".into());
            v.push("halfblocks".into());
        }
        v
    }

    /// The medium's absolute path.
    pub fn media_path(&self) -> PathBuf {
        repo_root().join(&self.media)
    }
}

// ── Provenance ────────────────────────────────────────────────────────────────

/// What the medium turned out to carry, read at capture time.
///
/// Measured, never declared: `load_mounted_story` mounts the file the way the
/// app does and the release and serial come out of the header of the bytes it
/// returned. A disk image is a different BUILD of the game, not the same story
/// on other media, so a caption that names a release it did not load is worse
/// than one that names none.
#[derive(Clone, Debug)]
pub struct Provenance {
    pub version: u8,
    pub release: u16,
    pub serial: String,
    /// The filesystem the mount reported, in prose, or "story file".
    pub medium: String,
    /// The v6 native screen in zvm pixels — the size the art is magnified FROM.
    /// `None` for every non-v6 story, which has no pixel screen to speak of.
    ///
    /// Derived like everything else here: this press's own picture space at this
    /// press's own art scale. It is not one number for the corpus — a Blorb press
    /// is 640x400, the standard Macintosh plate is 480x304, Arthur's Apple II
    /// press is 560x384 — so the pane size that magnifies it by a whole number
    /// is a per-shot answer and not a constant (SQ-0963).
    pub native: Option<(u32, u32)>,
}

impl Provenance {
    pub fn read(path: &Path) -> Result<Provenance, String> {
        let (loaded, image) = app::hints::load_mounted_story(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let bytes = loaded.bytes();
        if bytes.len() < 0x18 {
            return Err(format!("{}: too short to carry a Z-machine header", path.display()));
        }
        Ok(Provenance {
            version: bytes[0],
            release: u16::from_be_bytes([bytes[2], bytes[3]]),
            serial: String::from_utf8_lossy(&bytes[0x12..0x18]).into_owned(),
            medium: medium_name(image),
            native: (bytes[0] == 6).then(|| native_screen(path, image)).flatten(),
        })
    }

    /// `v6 r83/s890706 off a story file`.
    pub fn describe(&self) -> String {
        format!("v{} r{}/s{} off {}", self.version, self.release, self.serial, self.medium)
    }
}

/// The v6 native screen this press lays itself out on, in zvm pixels.
///
/// The chain is `startup.rs`'s and `session.rs`'s, written out rather than
/// approximated: the picture space through `std_window → native_std_window →
/// profile`, times the art scale that space is drawn at. CLAUDE.md is emphatic
/// that a harness which skips a rung of it measures a screen the player never
/// sees — Journey r77 and Arthur r63 are 560x384 presses that come out 640x400
/// if `native_std_window` is left off — and this number is the DENOMINATOR of
/// every magnification below, so getting it wrong would make each of them
/// self-consistently wrong.
fn native_screen(path: &Path, image: Option<app::hints::DiskImage>) -> Option<(u32, u32)> {
    let profile = app::interpreter::InterpreterProfile::resolve(path, None, None, image);
    let picts = app::graphics::PictSource::resolve(path, None);
    let space = picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let art_scale = picts.art_scale();
    // `session.rs`'s own rule: a declared picture space is drawn at the scale
    // this machine drew it; absent one there is nothing to scale and the
    // uniform doubling stands.
    let (aw, ah) = space.unwrap_or((320, 200));
    let (sx, sy) = match (space, art_scale) {
        (Some(_), Some(s)) => s,
        _ => (2, 2),
    };
    Some((u32::from(aw) * sx.max(1), u32::from(ah) * sy.max(1)))
}

fn medium_name(image: Option<app::hints::DiskImage>) -> String {
    use app::hints::DiskImage as D;
    match image {
        Some(D::Adf) => "an Amiga floppy",
        Some(D::Hfs) => "a Macintosh floppy",
        Some(D::Fat12Dos) => "a DOS floppy",
        Some(D::Fat12AtariSt) => "an Atari ST floppy",
        Some(D::ProDos) => "an Apple ProDOS floppy",
        Some(D::InfocomBootDisk) => "an Apple self-booting floppy",
        Some(D::CommodoreD64) => "a Commodore 1541 floppy",
        Some(D::Iso9660) => "an ISO 9660 CD-ROM",
        None => "a story file",
    }
    .to_string()
}

// ── Capturing one shot ────────────────────────────────────────────────────────

/// Everything a finished shot knows about itself — the record that goes into
/// `gallery.json` and under the picture.
#[derive(Clone, Debug)]
pub struct Taken {
    pub id: String,
    pub png: PathBuf,
    pub provenance: Provenance,
    pub cols: u16,
    pub rows: u16,
    pub cell_w: u16,
    pub cell_h: u16,
    pub turns: usize,
    pub seed: u32,
    pub backend: Backend,
    /// The face the glyphs were drawn with, named so a reader knows the type in
    /// the picture is the harness's and not a terminal's.
    pub face: String,
    /// The driver's own verdict on which protocol negotiated.
    pub verdict: String,
    /// How many boots it took to reach a frame that passed the shot's guard.
    /// More than one is worth seeing: it means this shot is timing-sensitive.
    pub attempts: usize,
    pub captured_bytes: usize,
    pub width: u32,
    pub height: u32,
    /// The v6 native screen this press lays out on, and how far the pane
    /// magnified it. `None` for a story with no pixel screen, and for the map
    /// shot whose pane is a split this file does not restate.
    pub native: Option<(u32, u32)>,
    pub magnification: Option<f64>,
    /// Characters neither the face nor the bitmap master could draw.
    pub unresolved_glyphs: Vec<char>,
}

impl Taken {
    /// `640x400 native, 2.000x` — or a complaint when the magnification is not a
    /// whole number, since that is the one thing about it worth reading.
    pub fn scale_note(&self) -> Option<String> {
        let (n, m) = (self.native?, self.magnification?);
        let whole = (m - m.round()).abs() < 1e-9 && m >= 1.0;
        Some(format!(
            "{}x{} native at {m:.3}x{}",
            n.0,
            n.1,
            if whole { "" } else { " (NOT a whole number — every edge in the art is interpolated)" }
        ))
    }
}

/// Boot lanthorn for one shot and hand back the capture, having first refused
/// every way the capture could be of the wrong thing.
pub fn capture(shot: &Shot, bin: &Path, work: &Path, timeout: std::time::Duration) -> Result<Capture, String> {
    let media = shot.media_path();
    if !media.exists() {
        return Err(format!("`{}`: no medium at {} (the media directories are gitignored)", shot.id, media.display()));
    }
    let (cols, rows) = shot.size_cells()?;
    let (cell_w, cell_h) = shot.cell_px();

    let user_dir = work.join(&shot.id);
    let _ = std::fs::remove_dir_all(&user_dir);
    std::fs::create_dir_all(&user_dir).map_err(|e| format!("`{}`: {e}", shot.id))?;
    // The seed goes in the global config rather than the per-game sidecar: the
    // sidecar is a bare-lines file the driver already owns for `show_map`, and
    // two writers of one file is how a shot silently loses its seed.
    std::fs::write(user_dir.join("config.toml"), format!("random_seed = {}\n", shot.seed))
        .map_err(|e| format!("`{}`: writing the pinned seed: {e}", shot.id))?;

    let mut spec = Spec::new(bin, &media, &user_dir);
    spec.cols = cols;
    spec.rows = rows;
    spec.cell_w = cell_w;
    spec.cell_h = cell_h;
    spec.hide_map = !shot.show_map;
    spec.keys = shot.keys()?;
    spec.timeout = timeout;
    spec.extra_args = shot.lanthorn_args();

    let cap = driver::run(spec).map_err(|e| format!("`{}`: {e}", shot.id))?;
    // A run cut short is a frame captured mid-script, and it looks exactly like
    // a frame captured on purpose — the keys the ceiling ate leave no mark on
    // the picture. Refuse it here rather than let the guard downstream report a
    // frame that "does not say" something the shot never got far enough to say.
    if cap.timed_out {
        return Err(format!(
            "`{}`: hit the {}s ceiling with keys still unsent — raise --timeout, or the key spec's \
             waits add up to more than it allows",
            shot.id,
            timeout.as_secs()
        ));
    }
    let neg = cap.negotiated();
    match shot.backend {
        // Not a warning. A capture that fell back measures a renderer the shot
        // did not ask for, and a gallery of the wrong renderer is worse than a
        // gallery with a hole in it.
        Backend::Kitty if !neg.is_kitty() => Err(format!("`{}`: {}", shot.id, neg.explain())),
        // The mirror of it: `--image-protocol halfblocks` was passed, so any APC
        // graphics at all means the flag did not take and this is not the frame
        // the manifest asked for.
        Backend::Halfblocks if neg.apc_commands > 0 => Err(format!(
            "`{}`: asked for half-blocks and got {} APC `_G` command(s) — the backend override did not take",
            shot.id, neg.apc_commands
        )),
        _ => Ok(cap),
    }
}

/// Every cell of the resolved screen as text, one line per row.
///
/// The kitty unicode placeholder is a cell carrying an image rather than a
/// glyph, so it reads as a space: an art cell is not text and must not look like
/// some to a substring search.
pub fn screen_text(res: &super::oracle::Resolved) -> String {
    let mut s = String::with_capacity(usize::from(res.rows) * (usize::from(res.cols) + 1));
    for row in 0..res.rows {
        for col in 0..res.cols {
            let ch = res.cell(row, col).ch;
            s.push(if matches!(ch, '\0' | '\u{10EEEE}') { ' ' } else { ch });
        }
        s.push('\n');
    }
    s
}

/// How many cells a placement would actually put pixels on.
pub fn art_cells(res: &super::oracle::Resolved) -> usize {
    let mut n = 0;
    for row in 0..res.rows {
        for col in 0..res.cols {
            if res.cell(row, col).image_id.is_some() {
                n += 1;
            }
        }
    }
    n
}

/// The non-vacuity guard: everything the shot said must be on screen, is.
///
/// A failure prints what IS on the screen, because "not the frame you asked for"
/// is only actionable next to the frame you got — and the fix is nearly always
/// another keypress rather than a weaker guard.
pub fn check_expectations(shot: &Shot, res: &super::oracle::Resolved) -> Result<(), String> {
    let text = screen_text(res);
    let missing: Vec<&str> = shot.expect.iter().map(|s| s.as_str()).filter(|w| !text.contains(*w)).collect();
    let art = art_cells(res);
    if missing.is_empty() && art >= shot.expect_art_cells {
        return Ok(());
    }
    let mut why: Vec<String> = Vec::new();
    if !missing.is_empty() {
        why.push(format!(
            "does not say {}",
            missing.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join(" or ")
        ));
    }
    if art < shot.expect_art_cells {
        why.push(format!("puts art on {art} cell(s), wanted at least {}", shot.expect_art_cells));
    }
    let seen: Vec<String> = text
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .take(12)
        .collect();
    Err(format!(
        "`{}`: the frame {} — this is not the screen the manifest asked for (a browser, a boot \
         prompt, or a different story off the same medium). What it does say, {} art cell(s) aside:\n{}",
        shot.id,
        why.join(", and "),
        art,
        if seen.is_empty() { "        (no text at all — an all-art frame)".to_string() } else { seen.iter().map(|l| format!("        {l}")).collect::<Vec<_>>().join("\n") }
    ))
}

// ── Type ──────────────────────────────────────────────────────────────────────

/// A glyph face for the gallery: the harness's own bitmap master, or a real
/// outline font loaded from disk.
///
/// The outline path exists only here. `raster::render`'s default is untouched
/// and the tests never reach this type, so the geometry oracle goes on looking
/// as synthetic as it should.
pub enum Face {
    /// [`app::render::bitfont`] — Uni-VGA 8x16, the face the v6 pixel composite
    /// itself draws with.
    Bitmap,
    /// A TrueType face rasterised at the size its own metrics put in the cell.
    Outline {
        name: String,
        font: Box<fontdue::Font>,
        px: f32,
        /// The cell this face's own metrics round to at `px`: `round(advance)`
        /// by `round(line height)`. Equal to the shot's cell when the size was
        /// chosen well, and worth printing when it is not.
        natural: (u32, u32),
        /// Every character neither this face nor the bitmap master could draw.
        ///
        /// The reason this quest exists is that a missing glyph is SILENT: the
        /// map's arrowheads came out as `.notdef` boxes under Monaco and the run
        /// reported nothing at all. A blank cell is quieter still, so the ones
        /// that get this far are counted and named at the end of the run.
        unresolved: std::cell::RefCell<BTreeSet<char>>,
    },
}

impl Face {
    /// Load a TTF and size it to the cell FROM THE FACE'S OWN METRICS.
    ///
    /// Every glyph in a terminal occupies exactly one cell, so the rasterisation
    /// that belongs here is the one whose natural line box IS the cell: `px =
    /// cell_h / new_line_size(1px)`. That used to be `cell_h * 0.78`, a constant
    /// that happens to be near the truth for some faces and not for others, and
    /// which quietly stretched or shrank the type against the cell it sat in.
    ///
    /// For the default face at the two cells this tool captures at, the answer is
    /// one of the sweet-spot sizes the quest names: 13px in an 8x16 cell, 16px in
    /// a 10x20 one (SQ-0963). A face with no horizontal line metrics at all keeps
    /// the old constant, because something has to be drawn.
    pub fn outline(path: &Path, cell_h: u16) -> Result<Face, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("font {}: {e}", path.display()))?;
        let font = fontdue::Font::from_bytes(bytes.as_slice(), fontdue::FontSettings::default())
            .map_err(|e| format!("font {}: {e}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "font".into());
        let per_px = font.horizontal_line_metrics(1.0).map(|m| m.new_line_size).filter(|v| *v > 0.0);
        let px = match per_px {
            Some(line) => (f32::from(cell_h) / line).round().max(1.0),
            None => f32::from(cell_h) * 0.78,
        };
        // The advance is the same for every glyph in a monospace face, so `M`
        // answers for all of them.
        let natural = (
            font.metrics('M', px).advance_width.round().max(1.0) as u32,
            per_px.map_or(u32::from(cell_h), |line| (line * px).round().max(1.0) as u32),
        );
        Ok(Face::Outline { name, font: Box::new(font), px, natural, unresolved: Default::default() })
    }

    /// How the label should name this face — which is the whole reason a real
    /// one is allowed at all.
    pub fn describe(&self) -> String {
        match self {
            Face::Bitmap => "Uni-VGA 8x16 (the harness's own bitmap face)".to_string(),
            Face::Outline { name, px, natural, .. } => {
                format!("{name} rasterised at {px:.0}px ({}x{}) by the harness", natural.0, natural.1)
            }
        }
    }

    /// Whether this face's own cell at its chosen size IS the cell it is drawn
    /// into. A complaint when it is not — never fatal, because a reader can still
    /// judge layout from slightly wrong type, but printed, because the whole
    /// reason a size is pinned is that nobody notices a drift by eye.
    pub fn cell_complaint(&self, cell_w: u16, cell_h: u16) -> Option<String> {
        match self {
            Face::Bitmap => None,
            Face::Outline { name, px, natural, .. } => (*natural != (u32::from(cell_w), u32::from(cell_h)))
                .then(|| {
                    format!(
                        "{name} at {px:.0}px has a {}x{} cell, but this shot is captured at {cell_w}x{cell_h} — \
                         the type will not sit square in it (SQ-0963 pins a face whose cell is exactly 1:2)",
                        natural.0, natural.1
                    )
                }),
        }
    }

    /// Characters no face in the chain could draw, in codepoint order.
    pub fn unresolved(&self) -> Vec<char> {
        match self {
            Face::Bitmap => Vec::new(),
            Face::Outline { unresolved, .. } => unresolved.borrow().iter().copied().collect(),
        }
    }

    /// Draw one glyph into a cell.
    pub fn draw(&self, canvas: &mut RgbaImage, ch: char, px: u32, py: u32, cw: u32, chh: u32, fg: Rgba<u8>) {
        match self {
            Face::Bitmap => app::render::bitfont::blit_glyph(canvas, ch, px, py, cw, chh, fg, None),
            Face::Outline { font, px: size, unresolved, .. } => {
                // The half-block and box-drawing glyphs are the picture's
                // STRUCTURE — rules, borders, and every pixel of a half-block
                // frame. A text face either lacks them or draws them with gaps
                // at the cell seams, so they stay with the bitmap master whose
                // cells tile exactly.
                //
                // And then the CAPABILITY question, which is the durable half
                // (SQ-0963). The old rule was a RANGE — U+2500..=U+259F — so the
                // map's arrowheads, which are Arrows and Geometric Shapes, went
                // to fontdue, which drew `.notdef`. Widening the range would fix
                // that one set of glyphs for that one face; asking the face
                // whether it HAS the glyph fixes it for every face anyone passes
                // to `--font`, including the ones nobody has thought of.
                if is_structural(ch) || !font.has_glyph(ch) {
                    app::render::bitfont::blit_glyph(canvas, ch, px, py, cw, chh, fg, None);
                    // The master is a short hand-authored list, not a font: it
                    // covers font 3, the ZSCII table and the runes, and nothing
                    // says it covers whatever the face just declined. Record what
                    // fell through both, so the next silent gap is a printed line
                    // rather than a blank cell somebody eventually notices.
                    if !is_structural(ch) && !ch.is_whitespace() && !app::render::bitfont::has_glyph(ch) {
                        unresolved.borrow_mut().insert(ch);
                    }
                    return;
                }
                let (m, bitmap) = font.rasterize(ch, *size);
                if m.width == 0 || m.height == 0 {
                    return;
                }
                // Baseline at 80% of the cell, glyph centred horizontally: a
                // terminal advances by the cell, not by the glyph's own width.
                let baseline = py as i64 + (i64::from(chh) * 4) / 5;
                let x0 = px as i64 + (i64::from(cw) - m.width as i64) / 2;
                let y0 = baseline - m.height as i64 - i64::from(m.ymin);
                for gy in 0..m.height {
                    let y = y0 + gy as i64;
                    if y < 0 || y >= i64::from(canvas.height()) {
                        continue;
                    }
                    for gx in 0..m.width {
                        let x = x0 + gx as i64;
                        if x < 0 || x >= i64::from(canvas.width()) {
                            continue;
                        }
                        let a = u32::from(bitmap[gy * m.width + gx]);
                        if a == 0 {
                            continue;
                        }
                        let dst = canvas.get_pixel(x as u32, y as u32).0;
                        let mix = |s: u8, d: u8| ((u32::from(s) * a + u32::from(d) * (255 - a)) / 255) as u8;
                        canvas.put_pixel(
                            x as u32,
                            y as u32,
                            Rgba([mix(fg[0], dst[0]), mix(fg[1], dst[1]), mix(fg[2], dst[2]), 255]),
                        );
                    }
                }
            }
        }
    }
}

/// Glyphs that are structure rather than type: half-blocks, shades, and the box
/// drawing range. These must tile with no seam, which only the bitmap master
/// does.
fn is_structural(ch: char) -> bool {
    matches!(ch, '\u{2500}'..='\u{259F}')
}

/// Monospace faces worth trying when `--font` was not given, in order. `~/`
/// means the user's home directory; nothing else is expanded.
///
/// **Fira Code leads, and it is a measurement rather than a taste** (SQ-0963).
/// A half-block sample is `cell_width` wide by `cell_height / 2` tall, so square
/// samples want a cell of exactly 1:2, and a face's cell is `round(advance · px)`
/// by `round(line · px)` — the two round at different rates, so what matters is
/// how often the ROUNDED cell lands on 2.000 rather than what the em ratio says.
/// Measured off the sfnt tables, over 6..24 px/em:
///
/// | face | advance/line (em) | ratio | rounded cells that hit 2.000 |
/// |---|---|---|---|
/// | Fira Code Nerd Font | 0.615 / 1.231 | **2.000** | 10 of 19 — 5x10, 6x12, 7x14, 8x16, 9x18, 10x20, 11x22, 13x26, 14x28, 15x30 |
/// | 0xProto Nerd Font Mono | 0.620 / 1.200 | 1.935 | — |
/// | Source Code Pro Nerd Font Mono | 0.600 / 1.257 | 2.095 | — |
/// | JetBrains Mono Nerd Font Mono | 0.600 / 1.320 | 2.200 | 1 of 19, at 4x8 |
/// | Monaco | 0.600 / 1.333 | 2.222 | — |
/// | Iosevka Term Nerd Font Mono | 0.500 / 1.250 | 2.500 | — |
///
/// Those ten sizes are the historical terminal cells, and [`Shot::cell_px`]
/// captures at two of them. JetBrains Mono — which this list led with, chosen on
/// glyph coverage back when coverage was load-bearing — is 10% off at every size
/// anyone would pick, so its shots sampled the artwork coarser down than across.
///
/// Coverage is no longer the deciding question, because [`Face::draw`] asks the
/// face whether it HAS each glyph and falls back to the bitmap master when it
/// does not. Worth knowing anyway: Fira Code does carry the map's arrowheads
/// (`↑ ↓ ▲ ▼ ◀ ▶`, verified against its `cmap`) and does NOT carry the portal
/// badges `⊙`/`⊗`, which JetBrains Mono did. Neither does the bitmap master, so
/// a frame containing one is named in the run's output rather than silently
/// losing it.
///
/// Deliberately short and platform-obvious. `.ttc` collections are skipped —
/// fontdue reads a single face — so this list is plain `.ttf` only.
pub const FONT_CANDIDATES: &[&str] = &[
    "~/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf",
    "/Library/Fonts/FiraCodeNerdFontMono-Regular.ttf",
    "~/.local/share/fonts/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/truetype/firacode/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/TTF/FiraCodeNerdFontMono-Regular.ttf",
    "/usr/share/fonts/truetype/firacode/FiraCode-Regular.ttf",
    "/System/Library/Fonts/Menlo.ttf",
    "/System/Library/Fonts/Monaco.ttf",
    "/System/Library/Fonts/Supplemental/Andale Mono.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
];

/// A candidate's path with a leading `~/` resolved against `$HOME`.
///
/// Nerd Fonts install per-user on both macOS and Linux — `~/Library/Fonts` and
/// `~/.local/share/fonts` — so a list of absolute paths could not name the face
/// this tool is supposed to lead with.
pub fn candidate_path(cand: &str) -> Option<PathBuf> {
    match cand.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)),
        None => Some(PathBuf::from(cand)),
    }
}

/// The first candidate that loads, or the bitmap face.
pub fn pick_face(explicit: Option<&Path>, cell_h: u16) -> Result<Face, String> {
    if let Some(p) = explicit {
        return Face::outline(p, cell_h);
    }
    for cand in FONT_CANDIDATES {
        let Some(p) = candidate_path(cand) else { continue };
        if p.is_file() {
            if let Ok(f) = Face::outline(&p, cell_h) {
                return Ok(f);
            }
        }
    }
    Ok(Face::Bitmap)
}

// ── The label ─────────────────────────────────────────────────────────────────

/// Height of one label line, in pixels.
const LABEL_LINE: u32 = 16;

/// Append a footer to `frame` saying, in the picture itself, what the picture is.
///
/// WHY IT IS BURNT IN AND NOT A CAPTION. An image gets separated from its page
/// the first time somebody drags it into a chat window, and the claim that
/// survives that trip is the one inside the pixels. The label is always drawn
/// with the BITMAP face, whatever the frame above it used: a footer that shares
/// the frame's typeface reads as part of the render, and this one has to read as
/// the harness talking about the render.
pub fn label(frame: &RgbaImage, lines: &[String]) -> RgbaImage {
    const PAD: u32 = 4;
    let w = frame.width().max(1);
    // WRAPPED, never clipped. A provenance line that runs off the right edge
    // loses the seed or the release, and the label's whole job is that those
    // travel with the picture.
    let cols = ((w.saturating_sub(PAD * 2)) / 8).max(8) as usize;
    let rows: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .flat_map(|(i, l)| wrap(l, cols).into_iter().map(move |r| (i, r)))
        .collect();

    let strip = LABEL_LINE * rows.len() as u32 + PAD * 2;
    let mut out = RgbaImage::from_pixel(w, frame.height() + strip, Rgba([18, 18, 20, 255]));
    for (x, y, p) in frame.enumerate_pixels() {
        out.put_pixel(x, y, *p);
    }
    // A hairline between the render and the harness's remark about it, so the
    // two are never read as one surface.
    for x in 0..w {
        out.put_pixel(x, frame.height(), Rgba([200, 40, 40, 255]));
    }
    for (n, (source, text)) in rows.iter().enumerate() {
        // The first source line is the disclaimer and is drawn in the divider's
        // own red; the rest is provenance in a quieter grey.
        let fg = if *source == 0 { Rgba([236, 130, 130, 255]) } else { Rgba([150, 152, 158, 255]) };
        let y = frame.height() + PAD + LABEL_LINE * n as u32;
        for (j, ch) in text.chars().enumerate() {
            app::render::bitfont::blit_glyph(&mut out, ch, PAD + j as u32 * 8, y, 8, LABEL_LINE, fg, None);
        }
    }
    out
}

/// Break `text` into runs of at most `cols` characters, at spaces where there is
/// one and mid-word where there is not.
fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let need = if line.is_empty() { word.chars().count() } else { line.chars().count() + 1 + word.chars().count() };
        if need > cols && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if word.chars().count() > cols {
            // One unbreakable token longer than the strip: cut it rather than
            // let it run off the edge.
            for chunk in word.chars().collect::<Vec<_>>().chunks(cols) {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                out.push(chunk.iter().collect());
            }
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// The two lines every gallery frame carries.
pub fn label_lines(t: &Taken) -> Vec<String> {
    vec![
        "RENDER, NOT A SCREENSHOT - honest about layout, art placement and colour; the type is the harness's".to_string(),
        format!(
            "{} | {} | {} | {}x{} cells at {}x{}px | {} | {} keypress(es) | seed {} | {}{} | lanthorn {}",
            t.id,
            t.provenance.describe(),
            t.face,
            t.cols,
            t.rows,
            t.cell_w,
            t.cell_h,
            t.backend.as_str(),
            t.turns,
            t.seed,
            t.png.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            // The magnification travels with the picture for the same reason the
            // release does: it is the difference between a frame whose art is
            // pixel-exact and one whose every edge was interpolated, and it is
            // not recoverable by looking (SQ-0963).
            t.scale_note().map(|s| format!(" | {s}")).unwrap_or_default(),
            buildinfo::LONG,
        ),
    ]
}

// ── The contact sheet ─────────────────────────────────────────────────────────

/// A plain HTML index over the frames, so the set can be reviewed in one place
/// before any of it reaches a page.
///
/// Not the website. This is a proof sheet: it exists so whoever regenerates the
/// gallery can see all of it at once and notice the frame that came out wrong.
pub fn contact_sheet(taken: &[Taken], failed: &[String]) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "<!doctype html><meta charset=\"utf-8\"><title>lanthorn gallery (proof sheet)</title>");
    let _ = writeln!(
        s,
        "<style>body{{background:#121214;color:#d8d8dc;font:14px/1.5 system-ui,sans-serif;margin:2rem}}\
         img{{max-width:100%;display:block;border:1px solid #333}}\
         figure{{margin:0 0 3rem}}figcaption{{margin-top:.5rem;color:#a0a0a8}}\
         .warn{{background:#3a1414;border:1px solid #7a2a2a;padding:1rem;margin-bottom:2rem}}\
         code{{color:#c8b48c}}</style>"
    );
    let _ = writeln!(
        s,
        "<div class=\"warn\"><strong>These are renders, not screenshots.</strong> Every frame below was \
         resolved out of the escape bytes the real lanthorn binary wrote to a pty. That makes them honest \
         about layout, art placement and colour, and about nothing else — the type is drawn by the harness, \
         not by anyone's terminal. Hero and marketing shots want a real terminal session.</div>"
    );
    let _ = writeln!(s, "<h1>lanthorn gallery</h1><p>lanthorn <code>{}</code>, {} frame(s).</p>", buildinfo::LONG, taken.len());
    if !failed.is_empty() {
        let _ = writeln!(s, "<div class=\"warn\"><strong>{} shot(s) did not produce a frame:</strong><ul>", failed.len());
        for f in failed {
            let _ = writeln!(s, "<li>{}</li>", escape(f));
        }
        let _ = writeln!(s, "</ul></div>");
    }
    for t in taken {
        let name = t.png.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let _ = writeln!(s, "<figure><img src=\"{}\" alt=\"{}\">", escape(&name), escape(&t.id));
        let _ = writeln!(
            s,
            "<figcaption><code>{}</code> — {} — {} — {}x{} cells, {} — {} keypress(es), seed {}{}{}</figcaption></figure>",
            escape(&t.id),
            escape(&t.provenance.describe()),
            escape(&t.face),
            t.cols,
            t.rows,
            t.backend.as_str(),
            t.turns,
            t.seed,
            t.scale_note().map(|n| format!(" — {}", escape(&n))).unwrap_or_default(),
            if t.unresolved_glyphs.is_empty() {
                String::new()
            } else {
                format!(
                    " — <strong>no glyph anywhere for {}</strong>",
                    escape(&t.unresolved_glyphs.iter().collect::<String>())
                )
            }
        );
    }
    s
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The regeneration record: what was captured, from what, at what size, with
/// what seed. The PNGs are output; THIS is the thing that says how to get them
/// back, and it sits beside them so a frame found on disk months later can be
/// traced to a build.
pub fn recipe_json(taken: &[Taken], manifest: &Path) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "  \"lanthorn\": {},", json_str(buildinfo::LONG));
    let _ = writeln!(s, "  \"manifest\": {},", json_str(&manifest.display().to_string()));
    let _ = writeln!(s, "  \"kind\": \"render — resolved from the escape stream the real binary emitted; not a screenshot\",");
    let _ = writeln!(s, "  \"shots\": [");
    for (i, t) in taken.iter().enumerate() {
        let _ = writeln!(s, "    {{");
        let _ = writeln!(s, "      \"id\": {},", json_str(&t.id));
        let _ = writeln!(s, "      \"png\": {},", json_str(&t.png.display().to_string()));
        let _ = writeln!(s, "      \"version\": {},", t.provenance.version);
        let _ = writeln!(s, "      \"release\": {},", t.provenance.release);
        let _ = writeln!(s, "      \"serial\": {},", json_str(&t.provenance.serial));
        let _ = writeln!(s, "      \"medium\": {},", json_str(&t.provenance.medium));
        let _ = writeln!(s, "      \"cols\": {}, \"rows\": {},", t.cols, t.rows);
        let _ = writeln!(s, "      \"cell_px\": [{}, {}],", t.cell_w, t.cell_h);
        let _ = writeln!(s, "      \"backend\": {},", json_str(t.backend.as_str()));
        let _ = writeln!(s, "      \"turns\": {},", t.turns);
        let _ = writeln!(s, "      \"seed\": {},", t.seed);
        let _ = writeln!(s, "      \"face\": {},", json_str(&t.face));
        let _ = writeln!(s, "      \"verdict\": {},", json_str(&t.verdict));
        let _ = writeln!(s, "      \"attempts\": {},", t.attempts);
        let _ = writeln!(s, "      \"captured_bytes\": {},", t.captured_bytes);
        match t.native {
            Some((w, h)) => {
                let _ = writeln!(s, "      \"native_px\": [{w}, {h}],");
            }
            None => {
                let _ = writeln!(s, "      \"native_px\": null,");
            }
        }
        match t.magnification {
            Some(m) => {
                let _ = writeln!(s, "      \"magnification\": {m:.6},");
            }
            None => {
                let _ = writeln!(s, "      \"magnification\": null,");
            }
        }
        let _ = writeln!(
            s,
            "      \"unresolved_glyphs\": {},",
            json_str(&t.unresolved_glyphs.iter().collect::<String>())
        );
        let _ = writeln!(s, "      \"png_px\": [{}, {}]", t.width, t.height);
        let _ = writeln!(s, "    }}{}", if i + 1 == taken.len() { "" } else { "," });
    }
    let _ = writeln!(s, "  ]");
    let _ = writeln!(s, "}}");
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
