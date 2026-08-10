//! Journey's boot passage lands at the top of the story panel — SQ-0755.
//!
//! The report, Journey under the Amiga profile in hybrid at a 159×61 pane: *"first
//! issue is that the intro text got rendered at the bottom overwriting the menu
//! blocks under Individual Commands"*, *"actualy a lot of the text is being
//! rendered down there"*, and later, with the frame captured: *"straigh boot,
//! rendering text to the wrong spot"*.
//!
//! WHERE SHOULD IT GO? Measured, under both interpreter profiles at the same pane,
//! rather than argued: Journey declares its story window at native (265,17) 368×272
//! the moment the play layout opens, and the IBM PC profile does the same thing at
//! (241,1) 392×304. Both games put the boot passage in the right-hand panel beside
//! the illustration, which is where the printed game does. So honouring the window
//! table at boot is NOT the defect — the defect is what ELSE was in that panel.
//!
//! THE CAUSE. A v6 `erase_window` never set `erase_lower_requested`; that flag is
//! the v1–5 lower window's. So on a v6 story the host was never told the game had
//! cleared its screen, and the transcript went on re-rendering every line the game
//! had ever printed into whatever the story window is NOW. Journey prints its title
//! block — `JOURNEY`, `Part One of`, the copyright, `[Press any key to begin]` —
//! while window 0 is the whole 640×400 screen, erases, opens the play layout, and
//! prints the intro into the panel; and the whole title block came back with it,
//! pushing the intro down the panel and, once the two together overflowed its 47
//! rows, hard against the command menu. Exactly the report, in both of its halves.
//!
//! The boundary now rides the same `ScreenClear` element that a prose window MOVED
//! out from under its own text already emits (SQ-0697), on its own machine field so
//! an erase never reads as a freeze — and not through `erase_lower`, because the app
//! TRUNCATES the transcript on a game-driven one (SQ-0407's menu-redraw collapse)
//! and every move in Journey is a keystroke.
//!
//! Driven the way `apply_game_driven_result` drives it, and rendered at the pane the
//! user captured, because Journey's prose never reaches the story window's `lines`:
//! it arrives as the session TRANSCRIPT and is drawn from `AppState`. A harness that
//! renders `session.screen()` without feeding the transcript renders a frame with no
//! story text in it at all, which is what three earlier sweeps of this frame were
//! measuring.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use app::engine::Engine;
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind, TurnResult};
use app::state::TranscriptKind;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

/// `zvm::screen::set_palette` is process-global, so the two profiles must not boot
/// side by side.
static PALETTE: Mutex<()> = Mutex::new(());

/// The pane from the user's own `/dump-windows` of the defective frame: 159×61 at
/// (1,1), cell 8×18, hybrid.
const USER_PANE: (u16, u16, u16, u16) = (1, 1, 159, 61);

/// A line of Journey's title block, printed while window 0 is the whole screen and
/// erased before the play layout opens. Nothing about it belongs in the story panel.
const TITLE_LINE: &str = "Part One of";

/// The first line of the boot passage — what the panel should be showing.
const INTRO_LINE: &str = "It was a Golden Age";

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn boot(profile: InterpreterProfile) -> Option<GameSession> {
    let story_path = stories_dir().join("journey-r83-s890706.z6");
    let story_bytes = match std::fs::read(&story_path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!("SKIP: gitignored story missing at {}", story_path.display());
            return None;
        }
    };
    zvm::screen::set_palette(profile.palette());
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&story_path).map(|(b, _)| b));
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px = picts.std_window().or_else(|| profile.std_window());
    let mut session = GameSession::new_with_trace(
        story_bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        profile.default_colours(),
        None,
    )
    .expect("Journey (v6) should load and boot without a ZError");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();
    Some(session)
}

/// What `turn::apply_game_driven_result` does with one turn's output — the path every
/// keystroke in Journey takes.
fn apply(state: &mut app::state::AppState, r: &TurnResult) {
    if r.transcript_elems.is_empty() {
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &r.transcript_runs);
    } else {
        app::state::apply_transcript_elems(state, &r.transcript_elems);
    }
}

#[allow(deprecated)]
fn fresh_state(honor: bool) -> app::state::AppState {
    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker =
        Some(ratatui_image::picker::Picker::from_fontsize(ratatui_image::FontSize::new(8, 18)));
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;
    state
}

/// Boot Journey and press through to the frame that carries the opening passage —
/// the STRAIGHT BOOT the report is about, not a driven-to-gameplay screen. Returns
/// the session and the app state its transcript accumulated on the way.
fn journey_at_boot_passage(profile: InterpreterProfile, honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let mut session = boot(profile)?;
    let mut state = fresh_state(honor);
    // The boot output, before any input — what `startup` pushes.
    let opening = session.take_transcript();
    state.push_transcript_runs(&opening, TranscriptKind::Story, &[]);
    for _ in 0..12 {
        if state.transcript.iter().any(|l| l.contains(INTRO_LINE)) {
            break;
        }
        let r = match session.pending_input() {
            InputKind::Line => session.submit(""),
            InputKind::Char => session.submit_char(13),
            InputKind::Event => session.submit(""),
        };
        apply(&mut state, &r);
    }
    Some((session, state))
}

fn render(
    state: &app::state::AppState,
    model: &app::engine::ScreenModel,
) -> (Rect, Buffer) {
    let area = Rect::new(USER_PANE.0, USER_PANE.1, USER_PANE.2, USER_PANE.3);
    let mut buf = Buffer::empty(Rect::new(0, 0, area.right() + 1, area.bottom() + 1));
    let _ = app::render::screen::render_story_pane(model, false, None, state, area, &mut buf);
    (area, buf)
}

fn pane_rows(buf: &Buffer, area: Rect) -> Vec<String> {
    (area.y..area.bottom())
        .map(|y| {
            (area.x..area.right())
                .map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' '))
                .collect()
        })
        .collect()
}

/// The story viewport this frame resolved, as `/dump-windows` records it.
fn viewport(state: &app::state::AppState) -> (u16, u16, u16, u16) {
    state
        .v6_cell_map
        .borrow()
        .iter()
        .find(|e| e.label == "viewport")
        .map(|e| e.cells)
        .expect("a hybrid ring frame records its story viewport")
}

/// SQ-0755 — the boot passage starts at the top of the story panel, and the title
/// block the game erased on its way there is gone.
///
/// Both halves are the same defect and both were reported. The passage sitting low in
/// the panel is what the title block above it pushed it to; "a lot of the text is
/// being rendered down there" is what that looks like once the two together overflow
/// the panel and the view bottom-sticks.
///
/// FALSIFY by dropping the erase boundary (`clears_host_prose` never set in
/// `exec.rs`):
///
/// > `Amiga honor=true: the story panel still carries Journey's erased title block —
/// > "Part One of" at pane row 7, which the game printed full-screen and erased before
/// > it opened this panel`
///
/// …and with that check lifted, the second half of the report, in the same run:
///
/// > `Amiga honor=true: the boot passage starts at pane row 18, 15 row(s) below the
/// > top of the story panel (rows 3..50) — the erased title block is standing above
/// > it`
#[test]
fn journey_boot_passage_starts_at_the_top_of_the_story_panel() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        for honor in [true, false] {
            let Some((session, state)) = journey_at_boot_passage(profile, honor) else { return };
            let model = session.screen();
            let (area, buf) = render(&state, &model);
            let ctx = format!("{profile:?} honor={honor}");
            let rows = pane_rows(&buf, area);
            let vp = viewport(&state);
            let (vy, vbot) = (vp.1, vp.1 + vp.3);

            // The passage is on screen at all, and inside the panel.
            let at = rows
                .iter()
                .position(|r| r.contains(INTRO_LINE))
                .map(|i| area.y + i as u16)
                .unwrap_or_else(|| panic!("{ctx}: the boot passage is on screen\n{}", rows.join("\n")));

            // …and the game's erased title block is not.
            if let Some(i) = rows.iter().position(|r| r.contains(TITLE_LINE)) {
                panic!(
                    "{ctx}: the story panel still carries Journey's erased title block — \
                     {TITLE_LINE:?} at pane row {}, which the game printed full-screen and erased \
                     before it opened this panel",
                    area.y + i as u16
                );
            }

            // …so the passage stands at the top of the panel, where the game printed it.
            assert!(
                at < vy + 3,
                "{ctx}: the boot passage starts at pane row {at}, {} row(s) below the top of the \
                 story panel (rows {vy}..{vbot}) — the erased title block is standing above it",
                at.saturating_sub(vy)
            );

            // …and none of it is down among the command menu, which is the shape the
            // report took: the panel's bottom edge is the last row any story prose
            // may occupy.
            let below: Vec<u16> = rows
                .iter()
                .enumerate()
                .filter(|(i, r)| area.y + *i as u16 >= vbot && r.contains(INTRO_LINE))
                .map(|(i, _)| area.y + i as u16)
                .collect();
            assert!(
                below.is_empty(),
                "{ctx}: story prose is drawn at pane row(s) {below:?}, below the story panel \
                 (rows {vy}..{vbot}) and over the command menu"
            );

            // …in the panel's own columns, not the picture's.
            let line = &rows[(at - area.y) as usize];
            let first = line.find(|c: char| !c.is_whitespace()).unwrap_or(0) as u16 + area.x;
            assert!(
                first >= vp.0,
                "{ctx}: the boot passage starts at pane column {first}, left of the story panel \
                 (column {}) — it is in the illustration's column",
                vp.0
            );
        }
    }
}

/// The companion measurement, and the reason the fix is not "put the boot text
/// somewhere else": BOTH profiles ask for the same thing.
///
/// The question the report could not answer from the frame alone was whether
/// Journey's opening passage is meant to run full width before the play layout
/// exists — in which case honouring a play-layout window table at boot would itself
/// be the defect. It is not: at the same pane, on the same boot, the IBM PC profile
/// declares the same right-hand panel (a wider one, at the screen's top edge) and
/// puts the same passage in it. The panel is where the boot text goes; what was
/// wrong was what stood above it.
#[test]
fn journey_declares_a_right_hand_story_panel_at_boot_under_both_profiles() {
    let _g: MutexGuard<()> = PALETTE.lock().unwrap_or_else(|e| e.into_inner());
    let mut seen = Vec::new();
    for profile in [InterpreterProfile::Amiga, InterpreterProfile::IbmPc] {
        let Some((session, state)) = journey_at_boot_passage(profile, true) else { return };
        let model = session.screen();
        let (area, buf) = render(&state, &model);
        let vp = viewport(&state);
        let native = state
            .v6_cell_map
            .borrow()
            .iter()
            .find(|e| e.label == "viewport")
            .map(|e| e.native)
            .expect("the viewport record carries the window's native box");
        // A right-hand panel: it starts past the middle of the game's 640px screen…
        assert!(
            native.0 >= 200,
            "{profile:?}: the boot story window is a right-hand panel — got native {native:?}"
        );
        // …and past the middle of the pane's picture column, in cells.
        assert!(
            vp.0 > area.x + area.width / 3,
            "{profile:?}: the story viewport sits right of the illustration — got {vp:?} in a \
             {}-column pane",
            area.width
        );
        // …and it is the passage's home.
        let rows = pane_rows(&buf, area);
        assert!(
            rows.iter().any(|r| r.contains(INTRO_LINE)),
            "{profile:?}: the boot passage is in that panel"
        );
        seen.push((profile, native, vp));
    }
    eprintln!("boot story panel, by profile: {seen:?}");
}
