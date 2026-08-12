//! Launch options: the boot-time choices a story can only be *started* with,
//! and the three doors that reach them (SQ-0789 / SQ-0791).
//!
//! # What belongs here, and the rule that decides it
//!
//! **An option is admitted only if it cannot be changed after boot.** That is
//! the entire argument for a launch dialog: a boot-time choice has nowhere else
//! to live, while anything the running app can already change belongs in the
//! settings screen, and duplicating it here would create two editors for one
//! value.
//!
//! Admitted:
//!
//! - **The picture archive.** Its pictures are resolved as the story starts, and
//!   swapping it under a running game would re-resolve resources beneath a VM
//!   that never learns it happened.
//! - **The interpreter number.** Header byte `0x1E` is read by the *story*
//!   (`crates/zvm/src/cpu/exec.rs` branches on `read_byte(0x1E) == 6`), so a game
//!   that has booted has already made decisions from it.
//!
//! Rejected, and why — because "any other options" is how a dialog becomes a
//! second settings screen:
//!
//! - **v6 render mode.** Checked rather than assumed: `/set-v6-render` switches
//!   hybrid/raster/frameless **live** (`SlashOutcome::SetV6Render`, applied to
//!   `state.config.v6_render` mid-session) and the settings screen persists it.
//!   It fails the admission rule outright.
//! - Colours, styles, map behaviour — all live-editable, all already have a home.
//!
//! # The one thing this module enumerates, and the line it must not cross
//!
//! [`discover_art_candidates`] lists every native archive sitting beside a story.
//! SQ-0734 rejected exactly that enumeration as an input to *resolution*, and
//! that rejection stands: the format carries no release number and no serial,
//! every Infocom Amiga release names its archive `Pic.data`, and a wrong pairing
//! is invisible — Arthur's plates drawn into Zork Zero look like art, not like an
//! error.
//!
//! **Discovery for DISPLAY is safe; discovery for PAIRING is not.** The list
//! below is safe for precisely the reason auto-pairing is not: it ends at a
//! human, who knows which game they own and can supply the assertion the file
//! format cannot make. Nothing in this module may ever be wired into
//! [`crate::graphics::PictureOverride::resolve`] or into any other automatic
//! choice. If you are here to "close the gap" by picking the best candidate
//! programmatically, that is the failure the tier policy exists to prevent.

use std::path::{Path, PathBuf};

use blorb::infocom_pics::{Flavour, InfocomPics};

// ── LaunchOverrides ───────────────────────────────────────────────────────────

/// The boot-time overrides one launch carries, ahead of anything on disk.
///
/// Empty is the ordinary case: every field `None` means "inherit", exactly as an
/// absent key in the per-game sidecar does. A field is `Some` only when a door
/// into this mechanism was actually used — `--pictures` on the command line, or
/// a value the launch-options dialog reports as *changed* from what this story
/// already inherits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOverrides {
    /// A native picture archive named for this launch. Outranks the per-game
    /// sidecar's `pictures` key — the more specific and more recent instruction
    /// wins, and the help text says so.
    pub pictures: Option<String>,
    /// An interpreter number for this launch (ZMSD §11.1.3), outranking both the
    /// per-game sidecar and the global config.
    pub interpreter_number: Option<u8>,
}

impl LaunchOverrides {
    /// Nothing overridden — the launch behaves exactly as it did before any of
    /// this existed.
    pub fn is_empty(&self) -> bool {
        self.pictures.is_none() && self.interpreter_number.is_none()
    }
}

// ── ArtCandidate ──────────────────────────────────────────────────────────────

/// One native picture archive found beside a story, described well enough to
/// choose by: what wrote it, how many pictures it holds, and which part of a
/// multi-part set it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtCandidate {
    /// Full path on disk.
    pub path: PathBuf,
    /// Bare filename — what goes into `pictures = "…"`, since the key resolves
    /// relative to the story's own directory.
    pub filename: String,
    /// Which platform's codec read it.
    pub flavour: Flavour,
    /// The rendition label a human recognises: `Amiga`, `MCGA`, `EGA`, `CGA`.
    pub rendition: &'static str,
    /// Directory entries — pictures plus size-only placeholders.
    pub pictures: usize,
    /// Part number; multi-part sets number their files 1, 2, …
    pub part: u8,
    /// The width of the picture space its coordinates use: 320 or 640.
    pub space_width: u16,
    /// Its name resembles this story's, so it is listed FIRST.
    ///
    /// A hint about ORDER and nothing else. It never filters (a story library is
    /// commonly flat — `stories/` holds Arthur, Journey, Shogun and Zork Zero
    /// side by side, so "beside the story" means "in with everything else", and
    /// hiding the non-matching ones would hide the very case tier 3 exists for:
    /// `FMVPOKER.EG1` is Zork Zero's archive under another name, and no stem rule
    /// could ever connect them). It never selects, either — that is precisely the
    /// stem-based auto-discovery SQ-0734 rejected on measured grounds. Sorting a
    /// list a human then reads is the one thing a guess this weak is good for.
    pub likely: bool,
}

impl ArtCandidate {
    /// The machine this rendition asks babelmap to present itself as.
    pub fn profile(&self) -> crate::interpreter::InterpreterProfile {
        crate::interpreter::InterpreterProfile::for_art_flavour(self.flavour)
    }

    /// An honest one-line caveat, or `None` when the rendition draws correctly.
    ///
    /// EGA and CGA store 640-wide art whose pixels are half as wide, and babelmap
    /// has no display mode for that yet (SQ-0790) — so it plots them one pixel
    /// per pixel and they come out twice as wide as they should. Saying so in the
    /// dialog is cheaper than letting someone pick `.eg1`, see a stretched plate,
    /// and wonder whether they chose the wrong file.
    pub fn caveat(&self) -> Option<&'static str> {
        (self.space_width == 640)
            .then_some("640-wide art with half-width pixels — not yet drawn at its true aspect")
    }
}

/// Extensions worth opening as a native Infocom archive. Case-insensitive.
///
/// `Pic.data` has no extension at all, so it is matched by *name* below — that
/// being the name every Infocom Amiga release uses, which is exactly why a stem
/// rule could never pair one automatically.
const ART_EXTS: &[&str] = &["pic", "mg1", "mg2", "eg1", "eg2", "cg1", "cg2", "data"];

/// Every native picture archive beside `story_path`, in a stable order (by
/// filename), each one actually parsed so the list can state its flavour,
/// picture count and part number rather than guess from a name.
///
/// **This is display-only.** See the module header: enumerating candidates is
/// safe because a person picks; nothing may consume this list automatically.
///
/// A file that does not parse is simply absent from the list — it is not a
/// candidate, and a name-shaped file that is not an archive (`saved.data`, say)
/// must not be offered as one. That silence is right *here* and wrong in
/// [`crate::graphics::PictureOverride::resolve`], where a named-but-unusable
/// archive is loud on purpose: this function answers "what could you pick?",
/// that one answers "you asked for this and it did not work".
pub fn discover_art_candidates(story_path: &Path) -> Vec<ArtCandidate> {
    let dir = match story_path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let story_key = name_key(story_path.file_stem().and_then(|s| s.to_str()).unwrap_or(""));
    let mut out: Vec<ArtCandidate> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        if !looks_like_art_name(&filename) {
            continue;
        }
        let Ok(raw) = std::fs::read(&path) else { continue };
        let Ok(pics) = InfocomPics::parse(raw) else { continue };
        let space_width = pics.picture_space_width();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        out.push(ArtCandidate {
            rendition: rendition_label(pics.flavour(), space_width, &filename),
            flavour: pics.flavour(),
            pictures: pics.entries().len(),
            part: pics.part(),
            space_width,
            likely: names_resemble(&story_key, &name_key(stem)),
            filename,
            path,
        });
    }
    // Resembling names first, then alphabetical within each group. Order only —
    // every candidate found is still in the list.
    out.sort_by(|a, b| {
        (!a.likely, a.filename.to_lowercase()).cmp(&(!b.likely, b.filename.to_lowercase()))
    });
    out
}

/// A filename stem reduced to its comparable head: lowercased, cut at the first
/// separator, and stripped of anything that is not a letter or digit.
///
/// `zork0-r393-s890714` → `zork0`; `Zork Zero - The Revenge…` → `zorkzero`.
fn name_key(stem: &str) -> String {
    stem.split(['-', '_', '.'])
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Do a story's name and an archive's resemble each other enough to sort the
/// archive to the top? A weak prefix test, on purpose: it only reorders a list a
/// person then reads, so a false positive costs a row's position and a false
/// negative costs nothing at all.
fn names_resemble(story: &str, art: &str) -> bool {
    const MIN: usize = 3;
    if story.len() < MIN || art.len() < MIN {
        return false;
    }
    story.starts_with(art) || art.starts_with(story)
}

/// Is this filename worth opening as a picture archive?
fn looks_like_art_name(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    if lower == "pic.data" || lower == "cpic.data" {
        return true;
    }
    match lower.rsplit_once('.') {
        // `.data` alone is far too broad to admit on its own; only the two
        // Infocom archive names above use it.
        Some((_, "data")) => false,
        Some((_, ext)) => ART_EXTS.contains(&ext),
        None => false,
    }
}

/// The rendition a human recognises.
///
/// The codec and the picture-space width are read from the file, so those two
/// are facts. Splitting the 640-wide PC case into EGA and CGA is *not* — the two
/// write the same container, and only Infocom's DOS 8.3 naming tells them apart.
/// That is a display label, never an input to anything, so leaning on the
/// extension here costs nothing; a 640-wide PC archive under some other name
/// says "EGA/CGA" and stays honest.
fn rendition_label(flavour: Flavour, space_width: u16, filename: &str) -> &'static str {
    match flavour {
        Flavour::AmigaMac => "Amiga",
        Flavour::Pc if space_width == 320 => "MCGA",
        Flavour::Pc => {
            let lower = filename.to_lowercase();
            if lower.ends_with(".cg1") || lower.ends_with(".cg2") {
                "CGA"
            } else if lower.ends_with(".eg1") || lower.ends_with(".eg2") {
                "EGA"
            } else {
                "EGA/CGA"
            }
        }
    }
}

// ── The derived interpreter number, and where it came from ────────────────────

/// Why the launch is advertising the interpreter number it is.
///
/// The dialog shows this beside the number because picking prettier art can
/// *move* it: SQ-0734 rules that a named archive's flavour selects the machine
/// unless a number is set explicitly. Silently changing the emulated machine
/// because someone chose an `.eg1` is the surprise this dialog exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterpreterSource {
    /// A number was set outright — in this dialog, on the command line, or in a
    /// config file. Beats everything.
    Explicit,
    /// Inferred from the flavour of the chosen picture archive.
    Artwork,
    /// Inferred from the medium: a story mounted out of an Amiga floppy.
    DiskImage,
    /// Nobody said anything: Frotz's rule, 6 for Version 6 and 1 otherwise.
    Default,
}

impl InterpreterSource {
    /// The provenance phrase shown after the number.
    pub fn label(self) -> &'static str {
        match self {
            InterpreterSource::Explicit => "set here",
            InterpreterSource::Artwork => "from the artwork",
            InterpreterSource::DiskImage => "from the disk image",
            InterpreterSource::Default => "default for this story",
        }
    }
}

/// The ZMSD §11.1.3 machine name for an interpreter number, or `"?"`.
///
/// Verified against the table quoted on `Cli::interpreter_number` (which is
/// itself the ZMSD's): 1 DECSystem-20, 2 Apple IIe, 3 Macintosh, 4 Amiga,
/// 5 Atari ST, 6 IBM PC, 7 Commodore 128, 8 Commodore 64, 9 Apple IIc,
/// 10 Apple IIgs, 11 Tandy Color.
pub fn interpreter_name(n: u8) -> &'static str {
    match n {
        1 => "DECSystem-20",
        2 => "Apple IIe",
        3 => "Macintosh",
        4 => "Amiga",
        5 => "Atari ST",
        6 => "IBM PC",
        7 => "Commodore 128",
        8 => "Commodore 64",
        9 => "Apple IIc",
        10 => "Apple IIgs",
        11 => "Tandy Color",
        _ => "?",
    }
}

/// The interpreter numbers the dialog offers, in cycling order: `None` (auto)
/// followed by the whole ZMSD table.
pub const INTERPRETER_CHOICES: [Option<u8>; 12] =
    [None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(6), Some(7), Some(8), Some(9), Some(10), Some(11)];

/// Resolve the interpreter number a launch would advertise, and say where it
/// came from — the same precedence [`crate::interpreter::InterpreterProfile::resolve`]
/// applies at boot, reported rather than applied.
///
/// `z_version` is the story's Z-machine version when known, because the default
/// rule depends on it (6 for Version 6, else 1). `None` — a Glulx or Scott story,
/// where header `0x1E` does not exist at all — yields no number to report.
pub fn derived_interpreter(
    explicit: Option<u8>,
    art: Option<&ArtCandidate>,
    disk_image: bool,
    z_version: Option<u8>,
) -> Option<(u8, InterpreterSource)> {
    if let Some(n) = explicit {
        return Some((n, InterpreterSource::Explicit));
    }
    let version = z_version?;
    if let Some(c) = art {
        // A profile that answers `None` is the IBM PC, which defers to zvm's own
        // rule rather than pinning a number — the same deferral, reported.
        let n = c.profile().interpreter_number().unwrap_or(if version == 6 { 6 } else { 1 });
        return Some((n, InterpreterSource::Artwork));
    }
    if disk_image {
        return Some((crate::interpreter::AMIGA_INTERPRETER_NUMBER, InterpreterSource::DiskImage));
    }
    Some((if version == 6 { 6 } else { 1 }, InterpreterSource::Default))
}

// ── LaunchOptionsState ────────────────────────────────────────────────────────

/// Which row of the dialog the cursor is on. Art rows come first (index 0 is
/// "no override"), then the interpreter row, then the persist checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// `0` = inherit (Blorb / disk image); `1..` indexes `candidates`.
    Art(usize),
    Interpreter,
    Persist,
}

/// Everything the launch-options dialog holds while it is open.
///
/// The two `baseline_*` fields are what this story *already* inherits with no
/// dialog involved. Every decision the dialog makes — what to apply to this
/// launch, and what the checkbox writes — is a comparison against them, which is
/// what keeps the per-game sidecar's "absent key = inherit" semantics intact: a
/// key written with the value it already inherits is **not** the same as a key
/// left absent, and this dialog never writes one.
#[derive(Debug, Clone)]
pub struct LaunchOptionsState {
    pub title: String,
    pub story_path: PathBuf,
    pub candidates: Vec<ArtCandidate>,
    /// `0` = inherit; `1..` selects `candidates[i - 1]`.
    pub art: usize,
    pub interpreter: Option<u8>,
    pub persist: bool,
    pub cursor: Row,
    /// Button focus ring: `0` = Play, `1` = Cancel.
    pub focus: usize,
    pub baseline_art: usize,
    pub baseline_interpreter: Option<u8>,
    /// The story's Z-machine version, for the default-rule half of the derived
    /// interpreter number. `None` for a non-Z story.
    pub z_version: Option<u8>,
    /// The story was mounted out of an Amiga release floppy.
    pub disk_image: bool,
}

/// What a key did to the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOptionsAction {
    /// Nothing the caller must act on.
    None,
    /// Start the story with these options.
    Play,
    /// Back out; launch nothing.
    Cancel,
}

impl LaunchOptionsState {
    /// Open the dialog for `story_path`, seeded with what it already inherits.
    ///
    /// `inherited_pictures` is the per-game sidecar's `pictures` key (`None` when
    /// absent), and `inherited_interpreter` the effective number in force before
    /// the dialog — sidecar first, then whatever the config/CLI settled.
    pub fn new(
        title: &str,
        story_path: &Path,
        inherited_pictures: Option<&str>,
        inherited_interpreter: Option<u8>,
        z_version: Option<u8>,
        disk_image: bool,
    ) -> LaunchOptionsState {
        let candidates = discover_art_candidates(story_path);
        // A sidecar naming an archive that is not in the list (an absolute path,
        // or a file that no longer parses) still deserves to be the baseline —
        // "inherit" is what it is, and the dialog must not silently re-point the
        // story at the Blorb just because it could not match the name.
        let art = inherited_pictures
            .and_then(|name| candidates.iter().position(|c| c.filename.eq_ignore_ascii_case(name)))
            .map_or(0, |i| i + 1);
        LaunchOptionsState {
            title: title.to_string(),
            story_path: story_path.to_path_buf(),
            candidates,
            art,
            interpreter: inherited_interpreter,
            persist: false,
            cursor: Row::Art(art),
            focus: 0,
            baseline_art: art,
            baseline_interpreter: inherited_interpreter,
            z_version,
            disk_image,
        }
    }

    /// The chosen archive, or `None` for "inherit".
    pub fn chosen_art(&self) -> Option<&ArtCandidate> {
        self.art.checked_sub(1).and_then(|i| self.candidates.get(i))
    }

    /// The number this launch would advertise, and where it came from.
    pub fn derived(&self) -> Option<(u8, InterpreterSource)> {
        derived_interpreter(self.interpreter, self.chosen_art(), self.disk_image, self.z_version)
    }

    /// Total selectable rows: one per art choice (plus "inherit"), the
    /// interpreter row, and the persist checkbox.
    pub fn row_count(&self) -> usize {
        self.candidates.len() + 3
    }

    /// The cursor as a flat row index.
    pub fn cursor_index(&self) -> usize {
        match self.cursor {
            Row::Art(i) => i,
            Row::Interpreter => self.candidates.len() + 1,
            Row::Persist => self.candidates.len() + 2,
        }
    }

    /// Move the cursor to a flat row index (clamped).
    pub fn set_cursor_index(&mut self, idx: usize) {
        let n = self.candidates.len();
        self.cursor = if idx <= n {
            Row::Art(idx)
        } else if idx == n + 1 {
            Row::Interpreter
        } else {
            Row::Persist
        };
    }

    /// The overrides this launch should carry: only what actually differs from
    /// what the story already inherits. An untouched dialog produces
    /// [`LaunchOverrides::default`], so opening it and pressing Play is
    /// indistinguishable from launching normally.
    pub fn overrides(&self) -> LaunchOverrides {
        LaunchOverrides {
            pictures: (self.art != self.baseline_art)
                .then(|| self.chosen_art().map(|c| c.filename.clone()))
                .flatten(),
            interpreter_number: (self.interpreter != self.baseline_interpreter)
                .then_some(self.interpreter)
                .flatten(),
        }
    }

    /// Did the user clear a choice they had inherited? Selecting "inherit" over a
    /// sidecar that names an archive cannot be expressed as an override (there is
    /// no "override with nothing"), so this launch honours it only via the
    /// checkbox — and the dialog says so rather than pretending otherwise.
    pub fn clears_inherited_art(&self) -> bool {
        self.art == 0 && self.baseline_art != 0
    }

    /// Same, for the interpreter number.
    pub fn clears_inherited_interpreter(&self) -> bool {
        self.interpreter.is_none() && self.baseline_interpreter.is_some()
    }

    /// Handle one keystroke.
    ///
    /// The model is the settings screen's, deliberately, because that is the
    /// idiom this project already edits values with: Up/Down move a row cursor,
    /// **Space** acts on the row under it, Left/Right cycle a row that has
    /// several values, Tab/Shift-Tab move button focus, Enter activates the
    /// focused button, Esc cancels. `config_screen_key_to_action` says it in one
    /// line — *"Enter activates the focused button; Space still toggles the
    /// selected row"* — which is what "Space is widget-reserved" means: the
    /// focused widget owns it, and dialog chrome must not.
    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> LaunchOptionsAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        let n = self.row_count();
        match key.code {
            KeyCode::Up => {
                self.set_cursor_index(self.cursor_index().saturating_sub(1));
                LaunchOptionsAction::None
            }
            KeyCode::Down => {
                self.set_cursor_index((self.cursor_index() + 1).min(n - 1));
                LaunchOptionsAction::None
            }
            KeyCode::Home => {
                self.set_cursor_index(0);
                LaunchOptionsAction::None
            }
            KeyCode::End => {
                self.set_cursor_index(n - 1);
                LaunchOptionsAction::None
            }
            KeyCode::Tab => {
                self.focus = crate::input::cycle_focus(self.focus, 2, 1);
                LaunchOptionsAction::None
            }
            KeyCode::BackTab => {
                self.focus = crate::input::cycle_focus(self.focus, 2, -1);
                LaunchOptionsAction::None
            }
            KeyCode::Left => {
                self.cycle(-1);
                LaunchOptionsAction::None
            }
            KeyCode::Right => {
                self.cycle(1);
                LaunchOptionsAction::None
            }
            KeyCode::Char(' ') if key.modifiers == KeyModifiers::NONE => {
                self.activate_row();
                LaunchOptionsAction::None
            }
            KeyCode::Enter => {
                if self.focus == 1 { LaunchOptionsAction::Cancel } else { LaunchOptionsAction::Play }
            }
            KeyCode::Esc => LaunchOptionsAction::Cancel,
            _ => LaunchOptionsAction::None,
        }
    }

    /// Space on the cursor row: select this archive, advance the interpreter by
    /// one, or flip the checkbox.
    fn activate_row(&mut self) {
        match self.cursor {
            Row::Art(i) => self.art = i,
            Row::Interpreter => self.cycle(1),
            Row::Persist => self.persist = !self.persist,
        }
    }

    /// Left/Right on the cursor row. Only rows with an ordered set of values
    /// respond; an art row is a radio button, not a cycler.
    fn cycle(&mut self, dir: isize) {
        match self.cursor {
            Row::Interpreter => {
                let cur = INTERPRETER_CHOICES.iter().position(|c| *c == self.interpreter).unwrap_or(0);
                let len = INTERPRETER_CHOICES.len();
                let next = (cur as isize + dir).rem_euclid(len as isize) as usize;
                self.interpreter = INTERPRETER_CHOICES[next];
            }
            Row::Persist => self.persist = !self.persist,
            Row::Art(_) => {}
        }
    }

    /// Write the checkbox's promise to `<game_dir>/config.toml`: the keys the
    /// user actually changed, and **only** those.
    ///
    /// A key the dialog merely *displayed* is never written. That is not tidiness
    /// — the sidecar's contract is "at most a few keys; absent key = inherit"
    /// (CLAUDE.md), so writing a key at the value it already inherits converts an
    /// inheritance into a pin, and the story stops tracking a later change to the
    /// global config. Only a difference from the baseline is a decision.
    pub fn persist_to(&self, game_dir: &Path) -> std::io::Result<()> {
        if self.art != self.baseline_art {
            crate::styles::write_per_game_pictures(
                game_dir,
                self.chosen_art().map(|c| c.filename.clone()),
            )?;
        }
        if self.interpreter != self.baseline_interpreter {
            crate::styles::write_per_game_interpreter_number(game_dir, self.interpreter)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stories_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("babelmap-launchopt-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn art_names_are_recognised_but_bare_dot_data_is_not() {
        assert!(looks_like_art_name("zork0.mg1"));
        assert!(looks_like_art_name("ZORK0.EG1"));
        assert!(looks_like_art_name("arthur.cg2"));
        assert!(looks_like_art_name("zork0.pic"));
        assert!(looks_like_art_name("Pic.data"));
        assert!(looks_like_art_name("CPIC.DATA"));
        // `.data` is far too common a suffix to admit on its own.
        assert!(!looks_like_art_name("saved.data"));
        assert!(!looks_like_art_name("zork0.z6"));
        assert!(!looks_like_art_name("Zork0.blb"));
        assert!(!looks_like_art_name("notes"));
    }

    #[test]
    fn a_name_shaped_file_that_is_not_an_archive_is_not_a_candidate() {
        // Display-only discovery must not offer something that cannot be used:
        // the list answers "what could you pick?", and an unparseable file is not
        // an answer. (A file NAMED in the sidecar is the opposite case, and stays
        // loud — see PictureOverride::resolve.)
        let dir = tmp("bogus");
        std::fs::write(dir.join("story.z6"), b"not a story").unwrap();
        std::fs::write(dir.join("story.mg1"), b"nowhere near an archive").unwrap();
        assert!(discover_art_candidates(&dir.join("story.z6")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zork_zeros_renditions_are_all_listed_with_their_flavours() {
        let z0 = stories_dir().join("zork0-r393-s890714.z6");
        if !z0.is_file() {
            return; // gitignored fixtures; skip vacuously (CI-safe)
        }
        let found = discover_art_candidates(&z0);
        let by_name = |n: &str| found.iter().find(|c| c.filename.eq_ignore_ascii_case(n)).cloned();
        // Every rendition the SQ-0734 note tabulates, each labelled from its own
        // codec and picture-space width rather than from its name.
        if let Some(c) = by_name("zork0.mg1") {
            assert_eq!(c.rendition, "MCGA");
            assert_eq!(c.flavour, Flavour::Pc);
            assert_eq!(c.space_width, 320);
            assert!(c.pictures > 100, "a real archive holds many pictures, got {}", c.pictures);
            assert!(c.caveat().is_none(), "MCGA draws correctly today");
        }
        if let Some(c) = by_name("zork0.eg1") {
            assert_eq!(c.rendition, "EGA");
            assert_eq!(c.space_width, 640);
            assert!(c.caveat().is_some(), "EGA must carry the SQ-0790 caveat");
        }
        if let Some(c) = by_name("zork0.pic") {
            assert_eq!(c.rendition, "Amiga");
            assert_eq!(c.flavour, Flavour::AmigaMac);
            assert!(c.caveat().is_none());
        }
        // The Blorb beside it is not a native archive and is never a candidate.
        assert!(by_name("Zork0.blb").is_none(), "a Blorb is tier 1, not a pickable archive");
    }

    #[test]
    fn an_untouched_dialog_overrides_nothing() {
        let dir = tmp("untouched");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let st = LaunchOptionsState::new("Story", &story, None, None, Some(6), false);
        assert_eq!(st.overrides(), LaunchOverrides::default());
        assert!(st.overrides().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_checkbox_writes_only_what_changed() {
        // The inherit contract: a key present at its inherited value is NOT the
        // same as a key absent, so a dialog that wrote every field it displayed
        // would quietly pin three settings the user never touched.
        let dir = tmp("persist");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let game_dir = dir.join("game");
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(6), false);

        // Nothing changed → nothing written; the sidecar is not even created.
        st.persist_to(&game_dir).unwrap();
        assert!(!crate::styles::per_game_config_path(&game_dir).exists());

        // Change only the interpreter → only that key lands.
        st.interpreter = Some(4);
        st.persist_to(&game_dir).unwrap();
        let body = std::fs::read_to_string(crate::styles::per_game_config_path(&game_dir)).unwrap();
        assert!(body.contains("interpreter_number = 4"), "got {body:?}");
        assert!(!body.contains("pictures"), "an untouched art choice must stay absent: {body:?}");
        assert!(!body.contains("honor_game_colours"), "unrelated keys stay absent: {body:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_derived_interpreter_reports_its_provenance() {
        // Explicit beats everything.
        assert_eq!(
            derived_interpreter(Some(3), None, true, Some(6)),
            Some((3, InterpreterSource::Explicit))
        );
        // A story with no Z version (Glulx/Scott) has no header 0x1E to report.
        assert_eq!(derived_interpreter(None, None, false, None), None);
        // The medium.
        assert_eq!(
            derived_interpreter(None, None, true, Some(6)),
            Some((4, InterpreterSource::DiskImage))
        );
        // Frotz's rule, both halves.
        assert_eq!(derived_interpreter(None, None, false, Some(6)), Some((6, InterpreterSource::Default)));
        assert_eq!(derived_interpreter(None, None, false, Some(5)), Some((1, InterpreterSource::Default)));
    }

    #[test]
    fn picking_amiga_art_moves_the_machine_and_the_dialog_can_say_so() {
        // The surprise this dialog exists to prevent: the art choice moves the
        // emulated machine unless a number is set explicitly.
        let amiga = ArtCandidate {
            path: PathBuf::from("/x/Pic.data"),
            filename: "Pic.data".into(),
            flavour: Flavour::AmigaMac,
            rendition: "Amiga",
            pictures: 172,
            part: 1,
            space_width: 320,
            likely: true,
        };
        let pc = ArtCandidate { flavour: Flavour::Pc, rendition: "MCGA", ..amiga.clone() };
        assert_eq!(
            derived_interpreter(None, Some(&amiga), false, Some(6)),
            Some((4, InterpreterSource::Artwork))
        );
        assert_eq!(
            derived_interpreter(None, Some(&pc), false, Some(6)),
            Some((6, InterpreterSource::Artwork))
        );
        // …and an explicit number still wins over the art.
        assert_eq!(
            derived_interpreter(Some(6), Some(&amiga), false, Some(6)),
            Some((6, InterpreterSource::Explicit))
        );
    }

    #[test]
    fn keys_follow_the_settings_screen_idiom() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let dir = tmp("keys");
        let story = dir.join("story.z6");
        std::fs::write(&story, b"x").unwrap();
        let mut st = LaunchOptionsState::new("Story", &story, None, None, Some(6), false);
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);

        // No candidates here, so rows are: Art(0), Interpreter, Persist.
        assert_eq!(st.row_count(), 3);
        assert_eq!(st.cursor, Row::Art(0));
        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Interpreter);
        // Right cycles the interpreter forward off auto; Left comes back.
        st.on_key(k(KeyCode::Right));
        assert_eq!(st.interpreter, Some(1));
        st.on_key(k(KeyCode::Left));
        assert_eq!(st.interpreter, None);
        // Space acts on the row under the cursor (the settings-screen rule).
        st.on_key(k(KeyCode::Char(' ')));
        assert_eq!(st.interpreter, Some(1), "Space advances the interpreter row");

        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Persist);
        assert!(!st.persist);
        st.on_key(k(KeyCode::Char(' ')));
        assert!(st.persist, "Space toggles the checkbox");
        st.on_key(k(KeyCode::Char(' ')));
        assert!(!st.persist);

        // Down at the end does not run off; Up/Home/End behave.
        st.on_key(k(KeyCode::Down));
        assert_eq!(st.cursor, Row::Persist);
        st.on_key(k(KeyCode::Home));
        assert_eq!(st.cursor, Row::Art(0));
        st.on_key(k(KeyCode::End));
        assert_eq!(st.cursor, Row::Persist);

        // Tab moves button focus and Shift-Tab reverses it (standing policy).
        assert_eq!(st.focus, 0);
        st.on_key(k(KeyCode::Tab));
        assert_eq!(st.focus, 1);
        assert_eq!(st.on_key(k(KeyCode::Enter)), LaunchOptionsAction::Cancel);
        st.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT));
        assert_eq!(st.focus, 0);
        assert_eq!(st.on_key(k(KeyCode::Enter)), LaunchOptionsAction::Play);
        assert_eq!(st.on_key(k(KeyCode::Esc)), LaunchOptionsAction::Cancel);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn selecting_an_archive_produces_exactly_one_override() {
        let z0 = stories_dir().join("zork0-r393-s890714.z6");
        if !z0.is_file() {
            return;
        }
        let mut st = LaunchOptionsState::new("Zork Zero", &z0, None, None, Some(6), false);
        let Some(idx) = st.candidates.iter().position(|c| c.filename.eq_ignore_ascii_case("zork0.mg1")) else {
            return;
        };
        st.art = idx + 1;
        let ov = st.overrides();
        assert_eq!(ov.pictures.as_deref(), Some("zork0.mg1"));
        assert_eq!(ov.interpreter_number, None, "an untouched interpreter row overrides nothing");
        // Seeding from that same sidecar value makes it the baseline, so the
        // dialog reopened on it overrides nothing at all.
        let st2 = LaunchOptionsState::new("Zork Zero", &z0, Some("zork0.mg1"), None, Some(6), false);
        assert_eq!(st2.art, idx + 1);
        assert!(st2.overrides().is_empty());
    }
}
