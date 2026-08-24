//! The typeface a release shipped on its own medium (SQ-1011).
//!
//! A companion to [`crate::graphics::PictSource`], and resolved the same way: the
//! MEDIUM is asked what it carries, and the answer is used only when it fits the
//! machine the medium named. Pictures were the first thing a release disk had that
//! beat lanthorn's own; the typeface is the second.
//!
//! # Why the Macintosh needed this and the others do not
//!
//! SQ-0917 gave the Macintosh its own Version 6 cell — **7x15**, which is what
//! `mac/xzip.lst` declares (`colWidth := 7; lineHeight := 15 {16}`) and what four
//! independent measurements off 1:1 captures confirm. That fixed the story's
//! layout and left its TEXT worse, because `crate::render::vga16` is drawn for an
//! **8-pixel advance**: 76 of its 94 printable glyphs ink out to column 6, so
//! column 7 is their entire inter-character gap and a 7-wide cell drops it. The
//! letters end up touching.
//!
//! The disk has the answer on it. Infocom shipped `FONT` 524 — 7x15, fixed-pitch
//! across printable ASCII, with left side bearings — which is exactly the cell,
//! so it blits **1:1 with no resampling on either axis**. That is the property
//! `vga16` was introduced for (SQ-0932) and precisely what it cannot deliver here.
//!
//! Every other machine still runs an 8x16 cell where `vga16` already blits 1:1,
//! so none of them reach this module.
//!
//! # The metric and the face are different facts
//!
//! Worth stating because they are easy to conflate: the Macintosh **declared** a
//! fixed 7 while **painting** proportional Geneva 12 — measured on
//! `machine-screenshots/mac-zorkzero-hint.png`, where the same `WING` glyph run
//! starts at x=137 in `EAST WING` and x=139 in `WEST WING`. `FONT` 524 is a
//! fixed-pitch resource the same release shipped. lanthorn renders on a fixed
//! cell, so the fixed face is the one that fits; see [`zvm::screen::V6Cell`] for
//! the declared-versus-drawn boundary this sits on.

use crate::interpreter::{InterpreterProfile, ProfileSource};
use crate::render::bitfont::STYLE_BOLD;
use blorb::bitmap_font::BitmapFont;
use std::path::Path;

/// The release's own face for `profile`'s cell, or `None` to keep `vga16`.
///
/// # Why `ProfileSource` gates this
///
/// The face lives in the mounted volume's resource fork, so it exists only when
/// the MEDIUM named the machine. `InterpreterProfile::resolve_with_source` reaches
/// `Macintosh` three ways and only one of them has a fork behind it:
///
/// * [`ProfileSource::Medium`] — an HFS volume is mounted. The font is there.
/// * [`ProfileSource::Asked`] — `--interpreter 3`, or `--pictures Pic.data` beside
///   a bare story file. A deliberate instruction from the player, with no volume
///   to read.
/// * [`ProfileSource::Fallback`] — never the Macintosh.
///
/// So the fallback path is not a fallback at all in ordinary use: a bare `.z6`
/// resolves to `IbmPc` and keeps its 8x16 cell, and the only way to hold a
/// Macintosh cell with no face is to have asked for one by hand.
///
/// **The CELL is not conditional on this.** What the story is told must not depend
/// on which glyphs the host happens to have; only the drawing does.
/// # And it is paired with ONE story, not with the disc
///
/// `entry` is which story on the image the session opened, as
/// [`crate::config::Config::disk_entry`] spells it. It matters because a
/// compilation carries many applications and only one of them is the game being
/// played — see [`blorb::mac_font::from_volume_beside`], and SQ-1018 for the
/// Masterpieces CD, where the first application on the platter ships no `FONT`
/// and every graphical game on it therefore drew its 7x15 cell with the 8-wide
/// fallback.
pub fn resolve(
    story_path: &Path,
    entry: Option<&str>,
    profile: InterpreterProfile,
    source: ProfileSource,
) -> Option<BitmapFont> {
    if source != ProfileSource::Medium {
        return None;
    }
    // An HFS volume keeps its faces in a resource fork; every other medium keeps
    // them as FILES, and Arthur's Amiga floppy is the second kind. Ask the volume
    // that can answer, and let `fit` decide whether what came back is usable.
    let face = match blorb::hfs::Hfs::mount(std::fs::read(story_path).ok()?) {
        Ok(hfs) => {
            // `entry` is `None` for every loose file and single-story floppy — and
            // also for a direct launch of a multi-game image, where `Hfs::story` is
            // the thing that CHOSE the story, so asking it again names the same one
            // rather than guessing. That is what makes
            // `lanthorn InfocomMasterpieces.img` pair correctly without a picker
            // row behind it.
            let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
            match opened {
                Some(p) => blorb::mac_font::from_volume_beside(&hfs, &p),
                None => blorb::mac_font::from_volume(&hfs),
            }
        }
        Err(_) => amiga_face(story_path),
    }?;
    fit(&face, profile).is_some().then_some(face)
}

/// The disk font an AmigaDOS volume carries, if it carries one.
///
/// Split out because it is the same lookup [`detected`] performs and the two must
/// not drift: SQ-1011 shipped inert twice over a fitness rule that existed in two
/// places, and a second copy of the LOOKUP would be the same defect one layer down.
fn amiga_face(story_path: &Path) -> Option<BitmapFont> {
    let files: Vec<(String, Vec<u8>)> = crate::assets::files(story_path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let name = f.name.clone();
            f.into_bytes().map(|b| (name, b))
        })
        .collect();
    blorb::amiga_font::from_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
}

/// How a release's own face may be drawn, where it may be drawn at all.
///
/// The two are not degrees of the same thing. [`FaceFit::Cell`] is a face that IS
/// the character cell and blits into it untouched; [`FaceFit::Metric`] is a face
/// whose own metrics REPLACE the cell's, and adopting it changes what the story is
/// told (SQ-1009). A caller has to know which it got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceFit {
    /// Same width, same height, one advance across printable ASCII — blit 1:1.
    /// The Macintosh's `FONT` 524.
    Cell,
    /// A proportional typeface: the pen advances per glyph and the line is the
    /// face's own height. Arthur's Amiga `char.data`.
    Metric,
}

/// Which of the two ways — if either — this face may be drawn on `profile`.
///
/// # `Cell`: the face IS the cell
///
/// This is the guard that keeps SQ-1011's fix from becoming the defect it replaces.
/// A fixed face drawn for a different advance has to be resampled into the cell,
/// and resampling is exactly what made `vga16` crowd at 7 wide. Better to keep the
/// known face than to introduce a second, differently-wrong one — so a mismatch
/// declines rather than scales. It also keeps the two facts honest about each
/// other: if a future profile moves its cell and the shipped face no longer
/// matches, this notices instead of silently drawing at the wrong pitch.
///
/// # `Metric`: the face brings its own
///
/// A PROPORTIONAL face has no single advance to match a cell against, so the test
/// above can never admit one and the resampling argument does not apply — there is
/// nothing to resample it to. What it has instead is a real advance per glyph and a
/// real line height, and a machine that shipped one drew text with them. So it is
/// admitted on being a typeface at all, and the cell follows it
/// ([`declared_cell`]) rather than the other way round.
///
/// **Proportional is also what separates a typeface from the font-3 SET**, which is
/// the trap this ordering avoids. Journey and Beyond Zork ship an 8x8 `Char.data` /
/// `Graphic.Data` that parses identically and is a box-drawing graphics set, not
/// letters — its code 65 is a solid block (SQ-1017). Both are fixed-pitch and carry
/// null `tf_CharSpace`/`tf_CharKern`, so both fall to the `Cell` test and are
/// declined there for not being the cell, exactly as they were before this existed.
///
/// # Why the printable range and not `BitmapFont::proportional`
///
/// That flag is measured over every non-blank glyph in the resource, and `FONT`
/// 524's Mac-roman accented range genuinely does vary — so it answers `true` for a
/// face that advances by exactly 7 across `!` to `~`, which is the only part a story
/// prints. SQ-0916 recorded this ("called proportional — true only if you count the
/// accented high range, which no game prints") and
/// `the_macintosh_font_is_fixed_pitch_but_narrower_than_our_cell` measures the
/// printable set as exactly `{7}`.
///
/// Gating on the flag is what made SQ-1011 ship INERT: the face resolved, failed
/// here, and the renderer silently kept `vga16` while four before/after frames came
/// back byte-identical. Ask the question the renderer actually depends on — does
/// every character a game prints advance by one cell. That same question, answered
/// the other way, is what now admits the Amiga's.
pub fn fit(face: &BitmapFont, profile: InterpreterProfile) -> Option<FaceFit> {
    let cell = profile.v6_font_cell();
    let uniform =
        (b'!'..=b'~').all(|c| face.glyph(c).is_none_or(|g| u16::from(g.width) == cell.w));
    if u16::from(face.width) == cell.w && u16::from(face.height) == cell.h && uniform {
        return Some(FaceFit::Cell);
    }
    (!uniform).then_some(FaceFit::Metric)
}

/// The Version 6 cell a machine declares once its own face has been admitted.
///
/// # The cell follows the FACE, and only when the face brings metrics
///
/// [`FaceFit::Cell`] and no face at all both answer `profile.v6_font_cell()`
/// unchanged, so every configuration that shipped before SQ-1009 lands on the same
/// number it always did. A [`FaceFit::Metric`] face is the one case where the
/// machine's table is not the whole story: it is a real typeface off the release's
/// own disk, drawn on its own line, and Arthur's Amiga floppy is the release that
/// has one.
///
/// # Why the HEIGHT moves and the WIDTH does not
///
/// The height is MEASURED. Arthur's `char.data` is 10 rows, `machine-screenshots/
/// amiga-arthur.png` reads a text pitch of exactly 10 in the machine's own 320x200
/// frame, and the four 2x captures read 20 — the same fact twice, and the reason a
/// scale multiplies it: a v6 coordinate is a NATIVE pixel, and on an Amiga press
/// whose art doubles onto the unit screen one FACE row is two native rows. For
/// Arthur that is a 20-row line against the 16 we declared, which is the 20 text
/// rows the machine shows against our 25.
///
/// **The scale is the TEXT scale and not the art scale** (SQ-1039). They coincide
/// on the Amiga, which draws its face in the picture space, and they do not on the
/// Macintosh, whose colour press doubles `CPic.data` onto the unit screen while
/// painting text at one native pixel per face pixel — so Geneva 12's fifteen rows
/// would be declared as thirty. `InterpreterProfile::text_scale` resolves it from
/// `zvm::interpreter::V6FaceSpace`; the monochrome press hides the difference,
/// because `Pic.data` is 480x300 at (1, 1) and 15 x 1 is still 15.
///
/// The width is NOT measured, and there is nothing here to measure. A proportional
/// face has no single advance — that is what makes it proportional — so any number
/// picked for `$27` would be a guess, and this repo does not guess declared
/// metrics (the Macintosh states `colWidth := 7` in `mac/xzip.lst` while painting
/// proportional Geneva; no equivalent Amiga listing survives, see SQ-1009). The
/// profile's width therefore stands, and what the story is TOLD about its columns
/// is unchanged. Only the DRAWING is proportional, which is the split
/// [`zvm::screen::V6Cell`] exists to name.
///
/// `art_scale` is [`crate::machine_boot::MachineBoot::art_scale`]; `None` there
/// means an undoubled rendition and should arrive here as `(1, 1)`. It is converted
/// to a text scale inside, so callers keep passing the ARCHIVE's number and only one
/// place knows the machine's rule.
pub fn declared_cell(
    profile: InterpreterProfile,
    face: Option<&BitmapFont>,
    art_scale: (u32, u32),
) -> zvm::screen::V6Cell {
    let cell = profile.v6_font_cell();
    match face.and_then(|f| fit(f, profile).map(|k| (f, k))) {
        Some((f, FaceFit::Metric)) => {
            // The TEXT scale, not the art scale (SQ-1039). They are the same number
            // on the Amiga — the only press with a typeface today, and one that
            // draws it in the picture space — and they are not on the Macintosh,
            // whose colour press doubles `CPic.data` while painting text at one
            // native pixel per face pixel. `art_scale.1` there would declare Geneva
            // 12's fifteen rows as thirty.
            let text = profile.text_scale(art_scale);
            zvm::screen::V6Cell::new(cell.w, u16::from(f.height).saturating_mul(text.1 as u16))
        }
        _ => cell,
    }
}

/// The cell, the face it is drawn with, and the pen that advances across it.
///
/// # Why these are one value
///
/// CLAUDE.md's refactoring policy: facts that must be considered TOGETHER travel
/// together, and the tell is a parameter list where two arguments always come from
/// the same place. `cell: V6Cell, face: Option<&BitmapFont>` were adjacent
/// parameters on five render functions and were supplied from the same two
/// [`crate::state::AppState`] fields at every one of the six call sites. Adding the
/// scale — which a [`FaceFit::Metric`] face needs and a `Cell` one does not — would
/// have edited all six again, which is when the omissions get made (SQ-0901,
/// SQ-1020, SQ-1021, SQ-1022, all the same shape).
///
/// # The pen
///
/// [`Self::advance`] is the whole difference between a machine that drew Arthur's
/// prose and lanthorn before SQ-1009. With no face, or a `Cell` face, it answers
/// the cell width for every character and every existing path behaves byte for
/// byte as it did. With a `Metric` face it answers that glyph's own advance times
/// the art scale, which is what `machine-screenshots/amiga-arthur-text.png`
/// measures to the pixel on three separate runs.
#[derive(Debug, Clone, PartialEq)]
pub struct TextFace {
    face: Option<BitmapFont>,
    fit: Option<FaceFit>,
    scale: (u32, u32),
    /// The declared cell and the pen as [`zvm`] holds them — see
    /// [`zvm::screen::V6Metric`].
    ///
    /// **The engine is handed THIS value, not a copy of the rule that built it**
    /// (SQ-1009). Both halves of the split measure the same text: the renderer
    /// asks [`Self::advance_styled`] where the next glyph goes, and zvm asks the
    /// same table where the cursor lands, where a line wraps and how wide a run
    /// came out for header `$30`. Two implementations of one advance would drift
    /// — SQ-1026 and SQ-1035 are a matched pair of exactly that — so there is
    /// one, and everything below delegates to it.
    metric: zvm::screen::V6Metric,
    /// Whether this machine rules under an emphasised run instead of sloping it
    /// (SQ-1028). A machine fact, carried here because `TextFace` is what already
    /// reaches every glyph the raster path draws.
    underline_emphasis: bool,
    /// A digest of everything above that moves a WRAP BOUNDARY: the declared cell
    /// and the advance of every byte the pen can measure (SQ-1034).
    ///
    /// Computed once here rather than compared per frame because the honest
    /// comparison — `PartialEq` on the whole face — walks a `BitmapFont`'s glyph
    /// bitmaps, and the wrap cache has to ask "has the face moved?" on every
    /// frame of both render paths. It is derived from the same `metric` the
    /// renderer and `zvm` both measure with, so a face that wraps differently
    /// cannot fingerprint the same.
    wrap_fp: u64,
}

/// Digest the wrap-relevant half of a resolved [`zvm::screen::V6Metric`]: the
/// cell and the advance of every byte. See [`TextFace::wrap_fingerprint`].
fn wrap_fingerprint_of(metric: &zvm::screen::V6Metric) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let cell = metric.cell();
    cell.w.hash(&mut h);
    cell.h.hash(&mut h);
    // Every §8.7.1 face the pen can be asked for, not just roman: the raster wrap
    // measures the run it is about to break, and an emphasised run is measured
    // with the emphasised advances.
    for style in [0u8, 2, 4, 6] {
        for b in 0u16..=255 {
            metric.advance(b as u8 as char, style).hash(&mut h);
        }
    }
    h.finish()
}

impl TextFace {
    /// Pair a resolved face with the machine that will draw it.
    ///
    /// The cell is [`declared_cell`]'s and never the caller's, so the three places
    /// that settle it — launch, style reload, `@restart` — cannot disagree.
    pub fn new(
        profile: InterpreterProfile,
        face: Option<BitmapFont>,
        art_scale: Option<(u32, u32)>,
    ) -> TextFace {
        // The TEXT scale (SQ-1039). Stored rather than the art scale because all
        // three consumers of `scale` are text: the declared cell, the advance table
        // below, and `render::bitfont`'s per-glyph blit. The artwork's own density
        // travels separately, in `AppState::v6_art_scale`.
        let scale = profile.text_scale(art_scale.unwrap_or((1, 1)));
        let fit = face.as_ref().and_then(|f| fit(f, profile));
        let cell = declared_cell(profile, face.as_ref(), scale);
        let metric = match (face.as_ref(), fit) {
            (Some(f), Some(FaceFit::Metric)) => {
                // Every byte carries a usable number, so `V6Metric` never has to
                // guess: a glyph the face does not cover falls back to the cell,
                // which is what the renderer draws it at.
                let mut advances = Box::new([cell.w; 256]);
                for (b, a) in advances.iter_mut().enumerate() {
                    if let Some(g) = f.glyph(b as u8) {
                        *a = (u32::from(g.width) * scale.0).min(u32::from(u16::MAX)) as u16;
                    }
                }
                let bold = (u32::from(f.bold_smear) * scale.0).min(u32::from(u16::MAX)) as u16;
                zvm::screen::V6Metric::proportional(cell, advances, bold)
            }
            _ => zvm::screen::V6Metric::fixed(cell),
        };
        let wrap_fp = wrap_fingerprint_of(&metric);
        TextFace { face, fit, scale, metric, underline_emphasis: profile.underlines_emphasis(), wrap_fp }
    }

    /// A cell with no release face behind it — a bare story, or a host default.
    pub fn cell_only(cell: zvm::screen::V6Cell) -> TextFace {
        let metric = zvm::screen::V6Metric::fixed(cell);
        let wrap_fp = wrap_fingerprint_of(&metric);
        TextFace { face: None, fit: None, scale: (1, 1), metric, underline_emphasis: false, wrap_fp }
    }

    /// A digest of everything about this face that can move a wrap boundary — the
    /// declared cell and every advance the pen answers. The transcript wrap cache
    /// keys on this rather than on the face itself (SQ-1034); see the field.
    pub fn wrap_fingerprint(&self) -> u64 {
        self.wrap_fp
    }

    /// What the STORY was told: [`zvm::screen::V6Cell`], declared metrics.
    pub fn cell(&self) -> zvm::screen::V6Cell {
        self.metric.cell()
    }

    /// The cell and the pen as the ENGINE takes them — `Machine::set_v6_text`.
    pub fn metric(&self) -> &zvm::screen::V6Metric {
        &self.metric
    }

    /// The face the renderer may draw with, if the release shipped a usable one.
    pub fn face(&self) -> Option<&BitmapFont> {
        self.face.as_ref()
    }

    /// How that face may be drawn — see [`FaceFit`].
    pub fn fit(&self) -> Option<FaceFit> {
        self.fit
    }

    /// Native pixels per face pixel, on each axis.
    pub fn scale(&self) -> (u32, u32) {
        self.scale
    }

    /// Whether an emphasised run is RULED rather than sloped — see
    /// [`crate::interpreter::InterpreterProfile::underlines_emphasis`].
    pub fn underlines_emphasis(&self) -> bool {
        self.underline_emphasis
    }

    /// Whether the pen advances per glyph rather than by a fixed cell.
    pub fn proportional(&self) -> bool {
        self.fit == Some(FaceFit::Metric)
    }

    /// Native pixels the pen moves for one ROMAN character — [`Self::advance_styled`]
    /// with no style byte.
    pub fn advance(&self, ch: char) -> u32 {
        self.advance_styled(ch, 0)
    }

    /// Native pixels the pen moves for one character drawn in ZMSD §8.7.1 `style`.
    ///
    /// A character the face does not cover falls back to the cell width, which is
    /// what every non-`Metric` configuration answers for everything.
    ///
    /// # Bold is WIDER, and that is not decoration
    ///
    /// The Amiga emboldens by smearing a glyph `tf_BoldSmear` pixels to the right
    /// and advancing the pen by the same amount, so the extra column has somewhere
    /// to live. Our synthesised bold smears without widening — at the old fixed
    /// 8-wide cell there was slack to absorb that, and at a real 3-to-8 px
    /// proportional advance there is none, so every bold glyph ate its own
    /// inter-character gap and bold words ran together. Arthur's `char.data`
    /// states a smear of **1** (SQ-1009).
    pub fn advance_styled(&self, ch: char, style: u8) -> u32 {
        u32::from(self.metric.advance(ch, style))
    }

    /// How far a run in `style` smears, in FACE pixels — 0 unless it is bold.
    pub fn bold_smear(&self, style: u8) -> u8 {
        match self.face.as_ref() {
            Some(f) if style & STYLE_BOLD != 0 => f.bold_smear,
            _ => 0,
        }
    }

    /// Native pixels a whole ROMAN run occupies — the pen's total, bearings and all.
    ///
    /// This is the width that WRAPS: `machine-screenshots/amiga-arthur-text.png`
    /// ends every full prose line within one word's width of the same pixel margin
    /// while carrying different character counts, which no column count reproduces.
    pub fn run_px(&self, s: &str) -> u32 {
        self.run_px_styled(s, 0)
    }

    /// [`Self::run_px`] for a run carrying a §8.7.1 style byte.
    pub fn run_px_styled(&self, s: &str, style: u8) -> u32 {
        self.metric.run_px(s, style)
    }

    /// Native pixels from one text baseline to the next — the cell's height.
    pub fn line_px(&self) -> u32 {
        u32::from(self.cell().h)
    }
}

/// One typeface a story's own medium carries, for the browser's info panel
/// (SQ-1018).
///
/// Display-only, exactly like [`crate::picker::StoryAux::art_candidates`] and
/// `disk_sounds`: it ends at a person's eyes and nothing downstream consumes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFace {
    /// How the medium names it — `FONT 524` on a Macintosh, the filename on an
    /// Amiga volume.
    pub name: String,
    /// The cell it is drawn for.
    pub width: u8,
    pub height: u8,
    /// Whether its advance actually varies — see [`BitmapFont::proportional`],
    /// and note it counts the accented range no game prints.
    pub proportional: bool,
    /// Whether [`resolve`] would hand THIS one to the renderer.
    pub used: bool,
}

/// Every face on the story's own medium, paired the way the renderer pairs them.
///
/// # Why this reports rather than re-deciding
///
/// `used` is settled by asking [`resolve`] and comparing the face it returns,
/// not by re-deriving which one wins. That costs a second mount and is worth it:
/// SQ-1011 shipped INERT TWICE because a fitness rule existed in two places and
/// correcting one left the other, and both false branches fell back silently. A
/// panel that decided for itself would be a third copy of the same question, and
/// the one a person would trust when it disagreed with the screen.
///
/// Had this surface existed, SQ-1018 would have been visible on sight rather
/// than reported as crowded text: the Masterpieces CD would have shown the face
/// present and unused.
pub fn detected(story_path: &Path, entry: Option<&str>) -> Vec<DiskFace> {
    let (profile, source) = InterpreterProfile::resolve_with_source(story_path, None, None, None);
    let chosen = resolve(story_path, entry, profile, source);
    let mark = |name: String, f: &BitmapFont| DiskFace {
        name,
        width: f.width,
        height: f.height,
        proportional: f.proportional,
        used: chosen.as_ref() == Some(f),
    };

    // A Macintosh names its faces, so report the ids: an id is family × 128 +
    // point size, which is what tells a reader that a release ships a body face
    // AND an alternate rather than two of the same thing.
    if let Some(hfs) = std::fs::read(story_path).ok().and_then(|b| blorb::hfs::Hfs::mount(b).ok()) {
        let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
        if let Some(p) = opened {
            let faces: Vec<DiskFace> = blorb::mac_font::faces_beside(&hfs, &p)
                .iter()
                .map(|(id, f)| mark(format!("FONT {id}"), f))
                .collect();
            if !faces.is_empty() {
                return faces;
            }
        }
    }

    // Every other medium: an AmigaDOS disk font is a file, so it is named by one.
    let files: Vec<(String, Vec<u8>)> = crate::assets::files(story_path)
        .into_iter()
        .filter(|f| f.is_on_medium())
        .filter_map(|f| {
            let name = f.name.clone();
            f.into_bytes().map(|b| (name, b))
        })
        .collect();
    blorb::amiga_font::faces_in_volume(files.iter().map(|(p, b)| (p.as_str(), b.as_slice())))
        .iter()
        .map(|(name, f)| mark(name.clone(), f))
        .collect()
}
