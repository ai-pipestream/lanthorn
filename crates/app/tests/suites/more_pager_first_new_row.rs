//! Where the `[more]` pager PARKS the view — SQ-0823.
//!
//! The arming rules are `more_pager_arming.rs`'s subject; this suite asks the one
//! question that comes after them, and that only a rendered frame can answer:
//! when the pager engages, is the FIRST new row the top row on screen?
//!
//! The report, playing Arthur: *"it scrolls one line too many before the [more]
//! prompt is shown"* — the pause comes a line late and a line of prose slips past.
//! It did, and the arithmetic was never the whole of it. `activation_target`
//! parked the view against the number the renderer called `viewport_rows`, and on
//! the frame the reader actually looks at, that number was not the count of rows
//! carrying prose:
//!
//! 1. the `[more]` bar takes a row out of the transcript for itself, and the park
//!    is computed one frame BEFORE it appears — so the row that would have been at
//!    the top is the row the bar displaces (Arthur, hybrid, shipped defaults);
//! 2. `viewport_rows` was the pane RECT, not the transcript body: a v3 status line
//!    (Cutthroats), the optional command bar, a suggestion strip. Each counted as a
//!    readable row, so a turn overflowing by exactly those rows raised no `[more]`
//!    at all and scrolled past in silence.
//!
//! Both are engine-neutral — the pager is shared by the Z-machine, Glulx and Scott
//! — so the cases below drive a v6 hybrid frame (Arthur, the reporter's game), the
//! Amiga release floppy of the same title (a different BUILD, release 54), the
//! same v6 game through the RASTER path (whose prompt is stamped over the last
//! prose row and so takes nothing — the fix must not move it), and a plain v3
//! story whose status line is the second defect in isolation.
//!
//! `stories/` is gitignored: a missing fixture skips vacuously, loudly on stderr.

use std::path::PathBuf;

use app::engine::Engine;
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use app::state::AppState;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot a v6 story the way the app does (pictures resolved, boot plates flushed)
/// and tap through its intro to ordinary play.
/// `release`/`serial` pin the BUILD the medium carries (`real_media_releases.rs`
/// is the authority for the table), so a failure can never be attributed to the
/// wrong one — a disk image is a different release, not the same story on other
/// media.
fn boot_v6(file: &str, release: u16, serial: &str, honor: bool) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let bytes = match app::hints::load_mounted_story(&path) {
        Ok((loaded, _)) => loaded.bytes().to_vec(),
        Err(_) => {
            eprintln!("SKIP: gitignored medium missing at {}", path.display());
            return None;
        }
    };
    assert_eq!(bytes[0], 6, "{file}: Z-machine version");
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file}: this medium carries a DIFFERENT build than the case says"
    );
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), serial, "{file}: serial");
    let mut picts = PictSource::resolve(&path);
    let picture_dims = picts.all_pict_dims();
    let std_win = picts.std_window();
    let mut s =
        GameSession::new_with_trace(bytes, honor, false, None, false, picture_dims, std_win, None, None)
            .expect("v6 story boots");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
    }
    Some(s)
}

fn story_state(honor: bool) -> AppState {
    let mut st = AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.config.honor_game_colours = honor;
    // The park is a scroll, and a scroll TWEENS: `effective_transcript_scroll`
    // reports a rounded intermediate while the ease is in flight, so a frame drawn
    // the instant after the park would be measured mid-animation, not at the
    // target. Every case here asks where the view SETTLES.
    st.config.animation.enabled = false;
    st
}

/// The outcome of one arm → frame → activate → frame cycle: what the pager did,
/// and what the frame the reader sees actually shows.
struct Parked {
    /// Absolute wrapped-row index of the first row this turn added.
    first_new_row: u16,
    /// Absolute wrapped-row index drawn at the TOP of the parked frame.
    top_visible_row: usize,
    /// Rows of prose the parked frame shows.
    viewport_rows: u16,
    /// Whether the `[more]` prompt is up.
    active: bool,
    /// Whether the parked frame really drew the `[more]` label.
    label_on_screen: bool,
}

/// Drive `cmds` as one turn's worth of output and run the run loop's own pager
/// sequence around it: cache the pre-turn total, arm, render (the frame that
/// decides), `apply_frame`, render again (the frame the reader sees).
fn park(s: &mut GameSession, state: &mut AppState, area: Rect, cmds: &[&str]) -> Parked {
    let render = |state: &AppState, s: &GameSession| {
        let mut buf = Buffer::empty(area);
        let m = app::render::screen::render_story_pane(&s.screen(), false, None, state, area, &mut buf);
        (m, state.transcript_geom.get(), buf)
    };

    state.push_transcript(&s.take_transcript());
    let (m0, _, _) = render(state, s);
    state.last_transcript_total_rows = m0.total_rows;
    // Rows `0..total` are old; row `total` is the first row this turn adds.
    let first_new_row = m0.total_rows;
    state.pager.arm(first_new_row);

    for c in cmds {
        let t = s.submit(c);
        state.push_transcript(&t.transcript);
    }

    let (m1, _, _) = render(state, s);
    app::pager::apply_frame(
        state,
        m1.max_scroll,
        m1.viewport_rows,
        m1.prompt_rows,
        m1.total_rows,
        m1.transcript_surface,
    );
    // The raster path builds its composite (and its metrics) on a worker thread and
    // redraws the last-ready one meanwhile; the run loop polls that job every tick,
    // so a harness that doesn't would measure the PREVIOUS frame's geometry.
    for _ in 0..400 {
        if state.poll_v6_encode_job() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (m2, g2, buf) = render(state, s);
    let g2 = g2.expect("the parked frame lays the transcript out");
    let label_on_screen = (0..area.height).any(|y| {
        let row: String =
            (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect();
        row.contains("[more]")
    });
    Parked {
        first_new_row,
        top_visible_row: g2.first_abs_row,
        viewport_rows: m2.viewport_rows,
        active: state.pager.active,
        label_on_screen,
    }
}

/// Assert the whole point: the pager engaged, and the reader's eye lands on the
/// first row of the new output with nothing of it above the fold.
///
/// `cell_prompt` says whether the `[more]` label should be findable as buffer TEXT
/// — true on the cell paths, false on the raster one, which blits it as glyph
/// pixels into the composite image where no cell carries it.
fn assert_nothing_scrolled_past(p: &Parked, cell_prompt: bool, ctx: &str) {
    assert!(p.active, "{ctx}: this turn overflows the pane — the pager must engage");
    assert_eq!(
        p.label_on_screen, cell_prompt,
        "{ctx}: an engaged pager draws its [more] prompt as terminal text on the cell paths only"
    );
    assert_eq!(
        p.top_visible_row, p.first_new_row as usize,
        "{ctx}: the FIRST new row must be the top row of the parked frame — it sits {} row(s) above \
         the fold, unread, which is the reported symptom (viewport {} rows)",
        p.top_visible_row as i64 - p.first_new_row as i64,
        p.viewport_rows,
    );
}

/// Arthur, hybrid, shipped defaults — the reported case, in both colour modes.
///
/// The `[more]` bar reserves the bottom row of the transcript for itself, and the
/// park is computed on the frame before it appears. Measured off
/// `arthur-r74-s890714.z6` at 80x30: 25 rows added into a 17-row viewport parked
/// at offset 8 and drew the first new row at absolute row 2 — one past the row the
/// turn started on. It parks at 9 now.
#[test]
fn arthur_hybrid_parks_the_first_new_row_at_the_top() {
    for honor in [true, false] {
        let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", honor) else { return };
        let mut state = story_state(honor);
        state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
        state.config.v6_render = app::config::V6RenderMode::Hybrid;
        let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look", "look"]);
        assert_nothing_scrolled_past(&p, true, &format!("arthur-r74-s890714.z6 hybrid, honor={honor}"));
    }
}

/// The same title on its Amiga release floppy — a different BUILD (release 54 /
/// serial 890606, against the story file's release 74), so the report is answered
/// on the medium as well as on the bare file.
#[test]
fn arthur_amiga_floppy_parks_the_first_new_row_at_the_top() {
    let file = "Arthur - The Quest for Excalibur.adf";
    let Some(mut s) = boot_v6(file, 54, "890606", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, &format!("{file} [release 54, serial 890606] hybrid"));
}

/// The RASTER path's `[more]` is stamped over the tail of the last prose row
/// rather than given a row of its own, so its viewport does not shrink when the
/// prompt shows — `prompt_rows` is 0 there and the park must stay where it was.
/// (The same fix applied blindly to both paths would push this one a row the wrong
/// way, showing a row of the PREVIOUS screen at the top instead.)
#[test]
fn arthur_raster_parks_the_first_new_row_at_the_top() {
    let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Raster;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look"]);
    assert_nothing_scrolled_past(&p, false, "arthur-r74-s890714.z6 raster");
}

/// The second half of the defect, with no `[more]` row involved at all: the pane
/// rect is not the transcript. Cutthroats is a v3 story, so its status line takes
/// the top row of the story pane — and the pager was handed the rect, status row
/// included. Measured at 80x30: a turn adding exactly 30 rows into a 29-row body
/// read as "fits, nothing missed", raised no prompt, and scrolled its first line
/// away.
#[test]
fn a_v3_status_line_is_not_a_readable_transcript_row() {
    let path = stories_dir().join("cutthroats-r23-s840809.z3");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return;
    };
    let mut s =
        GameSession::new_with_trace(bytes, true, false, None, false, Default::default(), None, None, None)
            .expect("Cutthroats boots");
    let mut state = story_state(true);
    let area = Rect::new(0, 0, 80, 30);

    // The status line's row is not a row the reader can read prose on.
    let mut buf = Buffer::empty(area);
    let m = app::render::screen::render_story_pane(&s.screen(), false, None, &state, area, &mut buf);
    assert_eq!(
        m.viewport_rows,
        area.height - 1,
        "premise: the v3 status line takes one of the pane's {} rows",
        area.height
    );

    let p = park(&mut s, &mut state, area, &["verbose", "look", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, "cutthroats-r23-s840809.z3 (v3 status line)");
}

/// …and so is the optional command bar. With `command_bar = true` the live input
/// gets its own bottom row, which the pager also counted as readable — a second
/// row of prose past the fold for anyone running that config.
#[test]
fn the_command_bar_row_is_not_a_readable_transcript_row() {
    let Some(mut s) = boot_v6("arthur-r74-s890714.z6", 74, "890714", true) else { return };
    let mut state = story_state(true);
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.command_bar = true;
    let p = park(&mut s, &mut state, Rect::new(0, 0, 80, 30), &["verbose", "look", "look"]);
    assert_nothing_scrolled_past(&p, true, "arthur-r74-s890714.z6 hybrid + command_bar");
}
