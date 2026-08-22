//! SQ-0954 — lanthorn's OWN annotation lines sit on the story window's page, not
//! on the machine's.
//!
//! **Reported by eye**: *"our info messages printed in story output (e.g. after a
//! `|` command) are not printed with the current story background, instead use the
//! machine background, which doesn't work well for Zork Zero."*
//!
//! # Measured cause
//!
//! `period::painted` gives three selectors the machine's PAGE — `transcript_input`
//! (the echoed command), `transcript_meta` (the gutter line) and
//! `transcript_warning`. That is deliberate and, for a v1–v5 story, right: those
//! lines are lanthorn's, their ink says something no machine has an opinion about,
//! but leaving their ground alone punches the host theme's page through the
//! machine's in the middle of the transcript.
//!
//! In v6 the machine's page need not be the ground. A game that calls `set_colour`
//! on window 0 declares its own, and Zork Zero does. Measured at 120x45, colours
//! honoured, with each medium's own period look applied:
//!
//! | medium                    | story page          | period page       | meta line's text sat on |
//! |---------------------------|---------------------|-------------------|-------------------------|
//! | `zork0-r393-s890714.z6`   | `Rgb(173, 173, 173)`| `Rgb(0, 0, 173)`  | **the period page**     |
//! | Amiga floppy (r366)       | `Rgb(173, 173, 173)`| `Rgb(7, 75, 161)` | **the period page**     |
//! | Macintosh disk (r296)     | *(none declared)*   | `Rgb(255,255,255)`| the period page — correct |
//!
//! The gutter GLYPH kept the story's grey throughout, because the symbol cell is
//! drawn with the marker style rather than the line style — so the reported row was
//! two grounds wide, a machine-coloured stripe with a story-coloured pip at its
//! left edge.
//!
//! **The Macintosh is why one machine is not enough.** There the story window
//! declares nothing, the machine's white IS the ground, and the frame is correct
//! before and after. A suite pinned to the reported title on its most obvious
//! medium would have shown no defect at all.
//!
//! # What is pinned
//!
//! The RELATIONSHIP, not a colour constant: **an annotation line is read on the
//! same ground as the prose above it**. Both `honor_game_colours` modes, per
//! CLAUDE.md — with colours declined the game never calls `set_colour`, so there is
//! no story page, the machine's is the ground, and nothing may move.
//!
//! The story media are gitignored, so every case **skips cleanly** when absent.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::{PictSource, PictureOverride};
use app::interpreter::InterpreterProfile;
use app::session::GameSession;
use app::state::{AppState, TranscriptKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The bare IBM PC story file: Zork Zero r393/890714, which boots
/// `set_colour(fg = 2 black, bg = 9 white)` on window 0 — so its story page is its
/// own and its period look is the IBM PC's blue. The widest gap of the three.
const IBM_PC_STORY: &str = "zork0-r393-s890714.z6";
/// The Amiga release floppy — a different build, r366/890323, and a different
/// machine page again.
const AMIGA_FLOPPY: &str = "Zork Zero - The Revenge of Megaboz.adf";
/// The Macintosh release disk, r296/881019: Zork Zero never calls `set_colour` on
/// it at all, so this is the CONTROL — the machine's page is the ground and the
/// frame must not move.
const MAC_DISK: &str = "Zork Zero Disk.image";

/// The text a case pushes as lanthorn's own, distinctive enough to find by eye in
/// a failure message.
const META: &str = "METAPROBE lanthorn speaking";

/// A pane roomy enough for the chrome ring plus a real story viewport.
const PANE: Rect = Rect { x: 0, y: 0, width: 120, height: 45 };

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

struct Frame {
    state: AppState,
    buf: Buffer,
    viewport: Rect,
}

/// Boot `file` the way `startup.rs` does — the medium picks the profile, the
/// archive has its say about colours, and the screen size comes off the mount —
/// then apply that machine's PERIOD LOOK, push one meta line, and render a hybrid
/// frame.
///
/// The period look is the whole point: without it these three selectors carry no
/// background at all and the cells keep whatever the pane painted, which is already
/// the story page. The defect only exists once a machine's page has been laid on
/// them, which is what `period::apply_to_theme` does on every real launch of a
/// licensed medium.
///
/// `None` when the gitignored medium is absent.
fn frame(file: &str, honor: bool) -> Option<Frame> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_story(&path) {
        Ok(app::hints::LoadedStory::ZCode(b)) => b,
        _ => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    let profile = InterpreterProfile::resolve(&path, None, PictureOverride::Unset.flavour(), None);
    app::v6_set_palette(profile.palette());
    let mut picts = PictSource::resolve_with_override(&path, PictureOverride::Unset, None);
    let picture_dims = picts.all_pict_dims();
    let std_window = picts
        .std_window()
        .or_else(|| picts.native_std_window())
        .or_else(|| profile.std_window());
    let honoured = honor && !picts.declines_game_colours(profile.default_colours());
    let mut session = GameSession::new_with_art_scale(
        bytes,
        honoured,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        std_window,
        picts.art_scale(),
        honoured.then(|| profile.default_colours()).flatten(),
        None,
        None,
    )
    .expect("Zork Zero should load and boot without a ZError");
    assert!(!session.quit && session.machine.fault_trace.is_none(), "{file} booted cleanly");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let mut state = AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honoured;
    let elems = Engine::take_transcript_elems(&mut session);
    app::state::apply_transcript_elems(&mut state, &elems);
    state.push_transcript_kind(META, TranscriptKind::Meta);

    // The machine's own screen, laid under the theme — `reload.rs` does exactly
    // this on every launch whose medium licenses it.
    if let Some(look) = app::period::resolve(profile, true, honoured, true, Some(6)) {
        app::period::apply_to_theme(&mut state.colors.theme, &look, Some(6));
        state.period_look = Some(look);
    }

    let model = session.screen();
    let mut buf = Buffer::empty(PANE);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, PANE, &mut buf);
    let viewport = state
        .transcript_geom
        .get()
        .expect("hybrid renders window 0 as a terminal transcript")
        .area;
    Some(Frame { state, buf, viewport })
}

/// The backgrounds carried by the drawn (non-blank) cells of the first row
/// containing `needle`, most common first. `None` when no row has it.
fn row_grounds(f: &Frame, needle: &str) -> Option<Vec<Color>> {
    let text = |y: u16| -> String {
        (f.viewport.x..f.viewport.right())
            .map(|x| f.buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
            .collect()
    };
    let y = (f.viewport.y..f.viewport.bottom()).find(|&y| text(y).contains(needle))?;
    let mut out: Vec<Color> = Vec::new();
    for x in f.viewport.x..f.viewport.right() {
        let c = f.buf.cell((x, y)).expect("in-bounds cell");
        if c.symbol() != " " && !out.contains(&c.bg) {
            out.push(c.bg);
        }
    }
    Some(out)
}

/// **The deliverable, as a relation.** On every medium and in both colour modes,
/// lanthorn's own meta line is read on the same ground as the story's prose.
///
/// The prose row is the room name the game just printed — a Story line with no
/// runs of its own, so its cells carry the pane's ground and nothing else. If the
/// meta line's ground differs from it, the player sees a stripe.
///
/// Falsified by reverting `render::transcript`'s re-grounding: the IBM PC frame
/// comes back `[Rgb(173, 173, 173), Rgb(0, 0, 173)]` against a prose ground of
/// `Rgb(173, 173, 173)` — the pip on the page, the sentence on the machine.
#[test]
fn a_meta_line_is_read_on_the_same_ground_as_the_prose() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any = false;
    for file in [IBM_PC_STORY, AMIGA_FLOPPY, MAC_DISK] {
        any |= stories_dir().join(file).exists();
        for honor in [true, false] {
            let Some(f) = frame(file, honor) else { continue };
            seen += 1;
            let what = format!("{file} honor={honor}");

            let prose = row_grounds(&f, "Banquet Hall")
                .unwrap_or_else(|| panic!("{what}: premise — the boot banner names the room"));
            assert_eq!(
                prose.len(),
                1,
                "{what}: premise — the prose row is one ground, or it cannot be the reference: {prose:?}",
            );
            let meta = row_grounds(&f, META)
                .unwrap_or_else(|| panic!("{what}: premise — the meta line reached the frame"));

            assert_eq!(
                meta, prose,
                "{what}: lanthorn's own line is read on a different ground from the prose above \
                 it — the machine's page punched through the page the story declared",
            );
        }
    }
    assert!(!any || seen > 0, "a present medium must have produced a frame");
}

/// **The direction**, and what stops the case above passing vacuously: the machine
/// really does lay a page on these selectors, and on the two media where the story
/// declares its own it is a DIFFERENT colour. Without this, a build that stopped
/// applying period looks at all would satisfy the relation trivially.
#[test]
fn the_period_look_really_does_paint_a_page_of_its_own() {
    let _g = app::v6_palette_at_boot();
    let mut seen = 0usize;
    let mut any = false;
    for (file, declares_its_own) in [(IBM_PC_STORY, true), (AMIGA_FLOPPY, true), (MAC_DISK, false)] {
        any |= stories_dir().join(file).exists();
        let Some(f) = frame(file, true) else { continue };
        seen += 1;
        let look = f.state.period_look.expect("a licensed medium at v6 has a period look");
        let page = Color::Rgb(look.page.0, look.page.1, look.page.2);
        let story = f.state.v6_story_page.get();
        assert_eq!(
            story.is_some(),
            declares_its_own,
            "{file}: premise — whether the story window declares a page of its own",
        );
        if let Some((r, g, b)) = story {
            assert_ne!(
                Color::Rgb(r, g, b),
                page,
                "{file}: premise — the two pages must DIFFER here, or this medium proves nothing",
            );
        }
    }
    assert!(!any || seen > 0, "a present medium must have produced a frame");
}
