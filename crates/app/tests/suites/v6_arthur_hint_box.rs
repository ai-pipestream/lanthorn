//! SQ-1006 — Arthur's boxed messages arrive by `print_form`, and the box was empty.
//!
//! # The report, and why it pointed the wrong way
//!
//! A lane driving Arthur to its InvisiClues screen noticed in passing that on
//! the in-play `hint` turn the game's answer — *"If only you had a crystal
//! ball...."* — never reaches [`GameSession::submit`]'s returned transcript,
//! while ordinary turns do, and guessed the trigger was the `@window_size` /
//! `@scroll_window` pair that turn issues on window 0.
//!
//! Both halves are false, and the second falsifies itself: `look` on the turn
//! before issues `@window_size(win=0, y=192, x=584)` too and its prose reaches
//! the transcript intact. Nor is *"not in the transcript"* a defect — the
//! message is a BOX. Arthur lays window 3 across the bottom of the screen and
//! prints there, and window 3 is not a flowing-prose window (wrap set,
//! scrolling clear — ZMSD §8.8.3.1), so its text paints at pixels the way a
//! status line does instead of streaming to the host's scrollback. The
//! transcript is the right place for it not to be.
//!
//! # What was actually wrong
//!
//! The turn's real shape, from the engine's own screen trace:
//!
//! ```text
//! @output_stream(3, table=0x4125, width=0)   ; justify as if in window 0
//! print "If only you had a crystal ball...."  ; captured, not printed
//! @output_stream(-3)                          ; close: table now holds it
//! …lay window 3 out across the bottom, erase it, colour it…
//! @print_form(table=0x4125)                   ; ← EXT:0x1A, and a no-op stub
//! ```
//!
//! `print_form` was unimplemented, so the message reached neither the screen
//! nor anywhere else: it existed only as bytes in a table nothing read. And the
//! table it would have read was the wrong SHAPE — ZMSD §15 `output_stream` says
//! that with a width operand "the table will contain not ordinary text but
//! formatted text: see print_form", and §15 `print_form` says that is "a
//! sequence of lines, terminated with a zero word", where "each line is a word
//! containing the number of characters, followed by that many bytes". zvm wrote
//! the plain §7.1.2.1 layout instead — one count word and the text after it.
//!
//! Arthur reads that table itself, and the wrong shape was visible in its own
//! geometry: it sized the box for **six** lines (96 px) for a 34-character
//! message, because it walked one bogus record after another before stumbling
//! onto a zero word. With the formatted layout it asks for one line (16 px),
//! which is what a 272-px message in a 584-px box needs.
//!
//! # The specimen
//!
//! `stories/Arthur - The Quest for Excalibur.adf`, release **54** / serial
//! 890606 — the Amiga floppy, booted the way `startup.rs` boots (profile from
//! the medium, screen through `std_window → native_std_window →
//! profile.std_window`). The frame is **sixteen turns in**: fourteen blank
//! turns past the intro answering `n` to the restore question, then `look`
//! (which is what leaves window 0 at its full 192 px, so the box has somewhere
//! to come from), then `hint`.
//!
//! Skip-if-missing (gitignored media), and non-vacuous: a fixture that is
//! present but yielded no frame fails rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::interpreter::InterpreterProfile;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

const FIXTURE: &str = "Arthur - The Quest for Excalibur.adf";
const RELEASE: u16 = 54;
const SERIAL: &str = "890606";

/// The game's own answer to `hint` in play, verbatim — four dots, not three.
const MESSAGE: &str = "If only you had a crystal ball....";

/// Panes swept. 80x25 is Arthur's own screen exactly (640x400 at an 8x16 cell);
/// 100x25 is the same height in a wider terminal, so the pair separates the
/// column mapping from the row mapping.
///
/// **Both are 25 rows deliberately, and that is not a round number** — at any
/// TALLER pane the hybrid-ring story viewport keeps its native rect (11 rows)
/// but takes every leftover cell row with it (21 rows at 80x34, 35 at 80x48),
/// overdrawing this box and every other v6 window below the story window. That
/// is SQ-1008, a render-layer defect this suite found and did not fix; pinning
/// a taller pane here would pin the bug rather than the fix.
const PANES: &[(u16, u16)] = &[(80, 25), (100, 25)];

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// Boot exactly as `startup.rs` boots — see the sibling suite's `boot` for why
/// every step of the chain matters (CLAUDE.md).
fn boot() -> Option<GameSession> {
    let path = stories_dir().join(FIXTURE);
    let (loaded, _) = app::hints::load_mounted_story(&path).ok().or_else(|| {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        None
    })?;
    let bytes = loaded.bytes().to_vec();
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), RELEASE, "{FIXTURE}: release");
    assert_eq!(String::from_utf8_lossy(&bytes[0x12..0x18]), SERIAL, "{FIXTURE}: serial");
    let profile = InterpreterProfile::resolve(&path, None, None, None);
    app::v6_set_palette(profile.palette());
    let mut picts = PictSource::resolve(&path, None);
    let picture_dims = picts.all_pict_dims();
    let v6_screen_px =
        picts.std_window().or_else(|| picts.native_std_window()).or_else(|| profile.std_window());
    let mut s = GameSession::new_with_art_scale(
        bytes,
        true,
        false,
        profile.interpreter_number(),
        false,
        picture_dims,
        v6_screen_px,
        picts.art_scale(),
        profile.default_colours(),
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{FIXTURE}: should boot without a ZError: {e:?}"));
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    Some(s)
}

/// Drive to the in-play `hint` turn and hand back the session together with
/// that turn's transcript. See the module docs for the route.
fn hint_in_play() -> Option<(GameSession, String)> {
    let mut s = boot()?;
    for _ in 0..14 {
        let r = match s.pending_input() {
            InputKind::Line => s.submit(""),
            InputKind::Char => s.submit_char(13),
            InputKind::Event => s.submit(""),
        };
        if r.transcript.to_lowercase().contains("y or n") {
            let _ = s.submit_char(b'n');
        }
        assert!(!s.quit, "{FIXTURE}: quit during the intro");
    }
    // `look` first: it is what leaves window 0 at its full height, so the turn
    // after it is a genuine "make room for a box" turn rather than a no-op.
    let r = s.submit("look");
    assert!(r.fault.is_none(), "{FIXTURE}: look faulted: {:?}", r.fault);
    assert!(
        r.transcript.contains("English churchyard"),
        "{FIXTURE}: the route must be standing in the churchyard, got {:?}",
        r.transcript
    );
    let r = s.submit("hint");
    assert!(r.fault.is_none(), "{FIXTURE}: hint faulted: {:?}", r.fault);
    Some((s, r.transcript))
}

/// The bottom box as the app publishes it: the one-row Grid the layered v6
/// model carries for window 3, with the pixel-positioned runs the game painted
/// into it.
fn box_runs(s: &GameSession) -> Vec<String> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { panic!("{FIXTURE}: a v6 frame is Layered") };
    items
        .iter()
        .filter_map(|it| match &it.node {
            WinNode::Grid(g) => Some(g.px_texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The pane's rows as chars — searched row by row rather than as one joined
/// string, because a kitty placeholder is four bytes and a byte offset is not a
/// column.
fn rows(buf: &Buffer, area: Rect) -> Vec<String> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')))
                .collect()
        })
        .collect()
}

fn state() -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    st.game_picker = Some(app::render::graphics::kitty_picker(8, 16));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st
}

/// **The report.** The message the game answers `hint` with reaches the screen:
/// the app publishes it as a painted run in the bottom box, and hybrid draws it
/// with glyphs at every pane size.
#[test]
fn the_hint_box_carries_the_games_answer() {
    let _g = app::v6_palette_at_boot();
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, _)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };

    let runs = box_runs(&s);
    assert!(
        runs.iter().any(|r| r == MESSAGE),
        "{FIXTURE} r{RELEASE}: the box must carry {MESSAGE:?} as one painted run — \
         `print_form` (EXT:0x1A) is how it gets there. Painted runs found: {runs:?}"
    );

    let mut checked = 0usize;
    for &(w, h) in PANES {
        let st = state();
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&s.screen(), false, None, &st, area, &mut buf);
        let lines = rows(&buf, area);
        assert!(
            lines.iter().any(|r| r.contains(MESSAGE)),
            "{FIXTURE} r{RELEASE} {w}x{h}: {MESSAGE:?} must be drawn with glyphs:\n{}",
            lines.join("\n")
        );
        checked += 1;
    }
    assert_eq!(checked, PANES.len(), "every pane measured");
}

/// The half of the report that was NOT a defect, pinned so nobody "fixes" it:
/// a boxed message is painted into window 3, and window 3 is not a flowing-prose
/// window, so it does not belong in the host transcript. The turn's transcript
/// is empty, and the turn before it — same `@window_size` on window 0 — is not.
#[test]
fn the_box_is_painted_and_stays_out_of_the_transcript() {
    let _g = app::v6_palette_at_boot();
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, transcript)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };
    assert!(
        !transcript.contains("crystal ball"),
        "{FIXTURE} r{RELEASE}: a boxed message is paint, not prose — it must not be spliced \
         into the scrolling transcript. Got {transcript:?}"
    );
    // …and it really is on the screen, so the assertion above is about ROUTING
    // and not about the message having gone missing again.
    assert!(
        box_runs(&s).iter().any(|r| r == MESSAGE),
        "{FIXTURE} r{RELEASE}: the box must still hold the message"
    );
}

/// Arthur reads the formatted table's own line records to decide how tall to
/// make the box, so the table's SHAPE is observable in the game's geometry: a
/// 34-character message justified to window 0's 584 px is one 16-px line. Six
/// lines (96 px) is what the plain §7.1.2.1 layout produced, walked as if it
/// were formatted text.
///
/// This is the case that fails on the unfixed engine even if `print_form` were
/// somehow satisfied another way — the two halves of the fix are independent
/// and both are needed.
#[test]
fn the_box_is_sized_for_one_line() {
    let _g = app::v6_palette_at_boot();
    let present = stories_dir().join(FIXTURE).exists();
    let Some((s, _)) = hint_in_play() else {
        assert!(!present, "{FIXTURE} is present but yielded no hint turn");
        return;
    };
    let v6 = s.machine.screen.v6.as_ref().expect("{FIXTURE}: a v6 screen");
    let boxw = &v6.windows[3];
    assert_eq!(
        (boxw.y_size, boxw.x_size),
        (16, 584),
        "{FIXTURE} r{RELEASE}: the game sizes its box from the formatted table's line records — \
         one line for a {}-character message in a 584px box",
        MESSAGE.chars().count()
    );
}
