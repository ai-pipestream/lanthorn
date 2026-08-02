//! A v6 game running TWO flowing-prose windows at once — SQ-0585.
//!
//! advent.z6's `style` command splits the screen into three: a text window across
//! the top (window 3, 180px), the status bar in the middle (window 1, y=181) and the
//! prose the player types into below (window 7, y=201). Windows 3 and 7 are both
//! wrap+scroll, so both stream through the Z-machine's stream-1 text path, and
//! babelmap used to splice them into ONE transcript — the top window's text then
//! scrolled away with the story. The game says so itself, on that very screen: "If
//! things scroll off of the screen that shouldn't (like the windows on the top of
//! the screen), then your interpreter probably doesn't support V6 correctly."
//!
//! The window the player types into keeps the transcript; any other prose window is
//! LIVE SCREEN STATE — its own buffer, no scrollback, cleared when the game erases
//! the window, and persisted with the rest of the screen so a restore reproduces it.
//!
//! Skip-if-missing per the other gitignored-story smokes.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot() -> Option<GameSession> {
    let story_path = stories_dir().join("advent.z6");
    let bytes = std::fs::read(&story_path).ok()?;
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let mut session =
        GameSession::new_with_trace(bytes, true, false, None, false, picture_dims, picts.std_window(), None)
            .expect("advent.z6 should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    let _ = session.take_transcript();
    for _ in 0..10 {
        match session.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                session.submit_char(13);
            }
            InputKind::Event => {
                session.submit("");
            }
        }
        let _ = session.take_transcript();
    }
    Some(session)
}

/// Drive to the split layout: one turn of play, then `style`.
fn split_open() -> Option<GameSession> {
    let mut session = boot()?;
    let _ = session.submit("look");
    let _ = session.take_transcript();
    let r = session.submit("style");
    assert!(!r.quit && r.fault.is_none(), "\"style\" faulted/quit: {:?}", r.fault);
    Some(session)
}

/// The BOOT banner still reaches the transcript. advent prints it into window 7
/// before it ever asks for input, so a rule that leaned only on "the window the
/// player types into" diverted the whole banner into a window buffer and the
/// transcript opened empty. ZMSD §8.8.3.1 attribute 2 — "text copied to output
/// stream 2" — is set on window 7 and settles it independently of input.
#[test]
fn the_boot_banner_still_reaches_the_transcript() {
    let Some(mut session) = boot() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    // `boot` drains the banner on the way to the first prompt, so re-boot raw.
    let story_path = stories_dir().join("advent.z6");
    let bytes = std::fs::read(&story_path).expect("story present");
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut fresh =
        GameSession::new_with_trace(bytes, true, false, None, false, dims, picts.std_window(), None)
            .expect("boot");
    fresh.set_pict_source(Some(picts));
    fresh.flush_boot_pictures();
    let banner = fresh.take_transcript();
    assert!(
        banner.contains("Version 6"),
        "the opening banner belongs to the transcript, got {} chars: {banner:?}",
        banner.len()
    );
    for (i, w) in fresh.machine.screen.v6.as_ref().unwrap().windows.iter().enumerate() {
        assert!(w.prose.is_empty(), "no window buffer swallowed the banner (window {i}: {:?})", w.prose);
    }
    let _ = session.take_transcript();
}

/// The engine keeps the second window's text in the WINDOW, not in the transcript
/// stream, and the split is decided by the game's own copy-to-transcript attribute.
#[test]
fn second_prose_window_keeps_its_own_text() {
    let Some(mut session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    // The `style` turn prints its explanation into window 3 — and nothing into the
    // transcript, which is exactly the point: it used to arrive there and scroll off.
    let turn = session.take_transcript();
    assert!(
        !turn.contains("You are standing at the end of a road"),
        "the top window's text must not reach the transcript, got {turn:?}"
    );

    let v6 = session.machine.screen.v6.as_ref().expect("v6 screen");
    assert_eq!(session.machine.v6_input_window, 7, "advent reads input through window 7");
    let top: String = v6.windows[3].prose.join(" ");
    assert!(
        top.contains("You are standing at the end of a road"),
        "window 3 holds its own prose, got {top:?}"
    );
    assert!(v6.windows[7].prose.is_empty(), "the input window streams to the transcript, not a buffer");

    // Playing on leaves the top window alone — the whole complaint was that it
    // scrolled away with the story.
    let before = v6.windows[3].prose.clone();
    let r = session.submit("look");
    assert!(!r.quit && r.fault.is_none(), "\"look\" faulted/quit");
    assert!(
        r.transcript.contains("You are standing at the end of a road"),
        "the story still streams to the transcript: {:?}",
        r.transcript
    );
    let after = session.machine.screen.v6.as_ref().unwrap().windows[3].prose.clone();
    assert_eq!(after, before, "the top window's text stays put across a turn");
}

/// The model publishes it as a NON-PRIMARY buffer at the window's own rect, beside
/// the primary story window — the same node Glulx uses for secondary text buffers.
#[test]
fn second_prose_window_reaches_the_model() {
    let Some(session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let buffers: Vec<(u16, u16, bool, usize)> = items
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Buffer(b) => Some((pw.y_px, pw.h_px, b.primary, b.lines.len())),
            _ => None,
        })
        .collect();
    let secondary = buffers.iter().find(|(_, _, primary, _)| !primary).expect("a secondary buffer");
    let primary = buffers.iter().find(|(_, _, primary, _)| *primary).expect("the story buffer");
    assert_eq!((secondary.0, secondary.1), (0, 180), "the top window at its own rect");
    assert!(secondary.3 > 0, "carrying its lines");
    assert_eq!((primary.0, primary.1), (200, 200), "the story window below it");
}

/// Rendered: the top window's text draws in the top window, the status bar sits
/// between, and the transcript draws below — all three at their own positions.
fn split_renders_three_regions(honor: bool) {
    let Some(session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let model = session.screen();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    for i in 0..6 {
        state.push_transcript(&format!("story line {i} ---------------"));
    }
    let area = Rect::new(0, 0, 80, 25);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let find = |needle: &str| (0..area.height).find(|&y| row(y).contains(needle));

    let top = find("You are standing at the end of a road")
        .unwrap_or_else(|| panic!("the top window renders its text (honor={honor})"));
    let bar = find("Score:").unwrap_or_else(|| panic!("the status bar renders (honor={honor})"));
    let story = find("story line 0").unwrap_or_else(|| panic!("the transcript renders (honor={honor})"));

    assert!(top < bar, "the top window is above the status bar (honor={honor}): top={top} bar={bar}");
    assert!(bar < story, "the status bar is above the story (honor={honor}): bar={bar} story={story}");
    // The top window sits in ITS window (0..180px = rows 0..11), not wherever the
    // transcript happens to flow.
    assert!(top < 11, "the top window's text stays inside its own window (honor={honor}): row {top}");
}

#[test]
fn split_renders_three_regions_honoring_game_colours() {
    split_renders_three_regions(true);
}

#[test]
fn split_renders_three_regions_theme_only() {
    split_renders_three_regions(false);
}

/// Live screen state, not history: erasing the window drops its lines.
#[test]
fn erasing_the_window_clears_its_prose() {
    let Some(mut session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    assert!(!session.machine.screen.v6.as_ref().unwrap().windows[3].prose.is_empty(), "prose present");
    // `help` erases the screen on its way in (measured: erase_window(-1) then a
    // fresh split), which must take the top window's lines with it.
    let _ = session.take_transcript();
    let _ = session.submit("help");
    assert!(
        session.machine.screen.v6.as_ref().unwrap().windows[3].prose.is_empty(),
        "an erase clears the window's live text"
    );
}

/// A restore reproduces the split: advent repaints NEITHER window afterwards
/// (measured), so the panel's content has to travel with the screen snapshot or it
/// comes back blank and stays blank.
#[test]
fn a_restore_brings_the_second_window_back() {
    let Some(session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let saved_screen = session.machine.screen.clone();
    let expected = saved_screen.v6.as_ref().unwrap().windows[3].prose.clone();
    assert!(!expected.is_empty(), "there is something to restore");

    // Round-trip the screen through the archive DTOs, as Save State does.
    let mapper = mapper::mapper::Mapper::default();
    let es = Engine::save_state(&session);
    let path = std::env::temp_dir().join(format!("advent-secondary-{}.babelmap", std::process::id()));
    app::archive::save_archive_meta_pics(
        &path,
        &mapper,
        &es,
        Some(&saved_screen),
        &session.machine.aux_data,
        app::archive::Meta {
            format_version: app::archive::CURRENT_FORMAT_VERSION,
            ifid: None, name: None, turns: 0, saved_at: String::new(),
            location: None, score: None, trigger: app::archive::SaveTrigger::HostState,
        },
        &[], &[], &[], &[], &[], &[], &[],
        &session.pictures_png(),
    )
    .expect("save_archive_meta_pics");
    let ac = app::archive::load_archive(&path).expect("load_archive");
    let _ = std::fs::remove_file(&path);

    let restored = ac.screen.as_ref().expect("persisted screen");
    assert_eq!(
        restored.v6.as_ref().unwrap().windows[3].prose,
        expected,
        "the second window's live text survives the archive round trip"
    );
}

/// SQ-0585: the chrome RING must not claim a secondary window's rows.
///
/// The ring is the area around the story viewport, carved into art and text strips.
/// A panel is neither — it is a text window the renderer draws itself — but with no
/// paint runs of its own its rows classified as ART, so the ring rasterized a slice
/// of the chrome canvas (which carries TEXT) straight over the panel. Under a
/// graphics protocol that image composites ABOVE the terminal cells, so the panel's
/// text vanished behind stray rasterized banner — invisible with a half-blocks
/// picker, which draws the same band as overwritable cells.
///
/// Assert on the strips through what reaches the buffer: the panel's own text, and
/// nothing from the transcript, on the panel's rows.
#[test]
fn the_ring_leaves_the_panels_rows_alone() {
    let Some(session) = split_open() else {
        eprintln!("SKIP: gitignored story missing");
        return;
    };
    let model = session.screen();
    // The panel's cell rows, from the model's pixel rect at this scale.
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    // Distinctive transcript text: if the ring rasterizes the chrome canvas over the
    // panel, banner-ish content lands on those rows.
    for i in 0..30 {
        state.push_transcript(&format!("TRANSCRIPTLINE{i} ----------------------------"));
    }
    let area = Rect::new(0, 0, 95, 49);
    let mut buf = Buffer::empty(area);
    let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);

    let row = |y: u16| -> String {
        (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect()
    };
    let panel_row = (0..area.height)
        .find(|&y| row(y).contains("You are standing at the end of a road"))
        .expect("the panel's text renders");
    // The panel sits above the status bar and the story.
    let bar_row = (0..area.height).find(|&y| row(y).contains("Score:")).expect("the bar renders");
    assert!(panel_row < bar_row, "panel above the bar: panel={panel_row} bar={bar_row}");

    // No transcript line appears on the panel's rows — the ring is not drawing there
    // and the story text is not bleeding up into it either.
    for y in 0..bar_row {
        let r = row(y);
        assert!(
            !r.contains("TRANSCRIPTLINE"),
            "row {y} is the panel's, but carries transcript text: {r:?}"
        );
    }
}
