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
    let image = std::fs::read(story_path).ok()?;
    let hfs = blorb::hfs::Hfs::mount(image).ok()?;
    // `entry` is `None` for every loose file and single-story floppy — and also
    // for a direct launch of a multi-game image, where `Hfs::story` is the thing
    // that CHOSE the story, so asking it again names the same one rather than
    // guessing. That is what makes `lanthorn InfocomMasterpieces.img` pair
    // correctly without a picker row behind it.
    let opened = entry.map(str::to_string).or_else(|| hfs.story().map(|(p, _)| p));
    let face = match opened {
        Some(p) => blorb::mac_font::from_volume_beside(&hfs, &p),
        None => blorb::mac_font::from_volume(&hfs),
    }?;
    fits(&face, profile).then_some(face)
}

/// A face is usable only when it IS the cell — same width, same height.
///
/// This is the guard that keeps the fix from becoming the defect it replaces. A
/// face drawn for a different advance has to be resampled into the cell, and
/// resampling is exactly what made `vga16` crowd at 7 wide. Better to keep the
/// known face than to introduce a second, differently-wrong one — so a mismatch
/// declines rather than scales.
///
/// It also means the two facts stay honest about each other: if a future profile
/// moves its cell and the shipped face no longer matches, this notices instead of
/// silently drawing at the wrong pitch.
fn fits(face: &BitmapFont, profile: InterpreterProfile) -> bool {
    let (cw, ch) = profile.v6_font_cell();
    if u16::from(face.width) != cw || u16::from(face.height) != ch {
        return false;
    }
    // **Uniform over PRINTABLE ASCII, which is not the same as `face.proportional`.**
    //
    // That flag is measured over every non-blank glyph in the resource, and `FONT`
    // 524's Mac-roman accented range genuinely does vary — so it answers `true`
    // for a face that advances by exactly 7 across `!` to `~`, which is the only
    // part a story prints. SQ-0916 recorded this ("called proportional — true only
    // if you count the accented high range, which no game prints") and
    // `the_macintosh_font_is_fixed_pitch_but_narrower_than_our_cell` measures the
    // printable set as exactly `{7}`.
    //
    // Gating on the flag is what made this feature ship INERT: the face resolved,
    // failed here, and the renderer silently kept `vga16` while four before/after
    // frames came back byte-identical. Ask the question the renderer actually
    // depends on — does every character a game prints advance by one cell.
    (b'!'..=b'~').all(|c| face.glyph(c).is_none_or(|g| u16::from(g.width) == cw))
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
