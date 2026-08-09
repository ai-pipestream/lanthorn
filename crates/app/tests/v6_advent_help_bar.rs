//! SQ-0727: advent's help bar dropped characters and clipped its tail, in cells.
//!
//! The user's screen, verbatim, hybrid mode at 120x40:
//!
//! ```text
//!                                     About Adventure
//!      N   n xt subj ct                                           P = previous
//!      RETURN = r ad subjec                                       Q   r sume game
//! ```
//!
//! The `=`, three lowercase `e`s and the final `t` of "subject", gone. It reads as
//! a glyph or encoding fault — lowercase `e` and `=` both vanishing while the `E`
//! of "RETURN" and both `e`s of "About Adventure" survive — and it is purely
//! positional.
//!
//! **A run is positioned by pixels and drawn by cells.** `draw_chrome_text_strip`
//! maps each run's native x to a terminal column with the letterbox scale, then
//! lays that run's characters out ONE COLUMN EACH. The two rates agree only when
//! the pane is exactly one column per native 8px text cell; at 120 columns of a
//! 640px screen a game cell is a column and a half, and they drift apart across
//! the row. advent paints each bar row as one label run PLUS the reversed blank
//! cells of the bar, and in native pixels every one of those blanks sits over
//! whitespace the label drew itself — harmless. Mapped, they no longer do:
//!
//! | native blank | maps to column | lands on |
//! |---|---|---|
//! | x=17  | 3  | the `=` of "N = next subject" |
//! | x=33  | 6  | the `e` of "next" |
//! | x=73  | 14 | the `e` of "subject" |
//! | x=113 | 21 | the `t` of "RETURN = read subjec**t**" |
//!
//! The last one is why the tail is clipped as well as the middle: x=113 is one
//! native cell PAST the label's last character in pixels, but inside its cell span
//! once the run is laid out a column per character. Interior drops and a clipped
//! tail, one mechanism, and it reproduced the user's screen exactly.
//!
//! A blank run carries no glyphs — the strip and row floods already put its
//! background down — so it now paints only the cells no text run claimed.
//!
//! The raster composite stays in native pixels throughout and was never wrong
//! here (`v6_raster_text_loss.rs`, confirmed on a real terminal); 80 columns is the
//! width where the two rates coincide, and it is pinned below so a fix cannot trade
//! one width for another.
//!
//! Both `honor_game_colours` modes are pinned: advent colours none of these runs,
//! so a mode-specific regression would otherwise hide. The story is gitignored
//! (CLAUDE.md), so these skip cleanly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

fn advent_help(honor: bool) -> Option<(GameSession, app::state::AppState)> {
    let path = stories_dir().join("advent.z6");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: gitignored story missing at {}", path.display());
        return None;
    };
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let mut session = GameSession::new_with_trace(
        bytes, honor, false, None, false, dims, picts.std_window(), None, None,
    )
    .expect("advent.z6 is a valid v6 story");
    session.set_pict_source(Some(picts));
    session.flush_boot_pictures();

    let mut state = app::state::AppState::default();
    state.colors = app::colors::ColorScheme::terminal_default();
    state.game_picker = Some(ratatui_image::picker::Picker::halfblocks());
    state.config.v6_render = app::config::V6RenderMode::Hybrid;
    state.config.honor_game_colours = honor;

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
    }
    app::state::apply_transcript_elems(&mut state, &Engine::take_transcript_elems(&mut session));
    let r = session.submit("help");
    assert!(r.fault.is_none(), "advent faulted opening help: {:?}", r.fault);
    app::state::apply_transcript_elems(&mut state, &r.transcript_elems);
    Some((session, state))
}

/// One rendered pane row, as text.
fn row_text(buf: &Buffer, width: u16, y: u16) -> String {
    (0..width).map(|x| buf.cell((x, y)).map(|c| c.symbol().chars().next().unwrap_or(' ')).unwrap_or(' ')).collect()
}

/// Every label of the bar, whole, at four pane widths — including the ones where a
/// native text cell is not a whole number of terminal columns.
fn advent_help_bar_keeps_every_character(honor: bool) {
    let Some((session, state)) = advent_help(honor) else { return };

    // Premise: the game really does paint the bar as label runs PLUS reversed blank
    // cells that fall inside those labels' pixel spans — the table above.
    let model = session.screen();
    let WinNode::Layered(items) = &model.root else { panic!("v6 Layered root") };
    let runs: Vec<(u16, u16, String)> = items
        .iter()
        .filter_map(|pw| match &pw.node {
            WinNode::Grid(g) => Some(g.px_texts.iter()),
            _ => None,
        })
        .flatten()
        .map(|t| (t.x, t.y, t.text.clone()))
        .collect();
    assert!(
        runs.contains(&(9, 17, "N = next subject".into())),
        "premise (honor={honor}): the label is ONE run at native (9,17): {runs:?}"
    );
    for x in [17u16, 33, 73] {
        assert!(
            runs.contains(&(x, 17, " ".into())),
            "premise (honor={honor}): a blank run sits at native x={x}, inside that label's span"
        );
    }
    assert!(
        runs.contains(&(113, 33, " ".into())),
        "premise (honor={honor}): a blank run sits at native x=113 — past the pixel end of \
         \"RETURN = read subject\", inside its cell span"
    );

    for (w, h) in [(80u16, 25u16), (100, 30), (120, 40), (160, 50)] {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let _ = app::render::screen::render_story_pane(&model, false, None, &state, area, &mut buf);
        let rows = [row_text(&buf, w, 0), row_text(&buf, w, 1), row_text(&buf, w, 2)];
        for (row, text, label) in [
            (0usize, "About Adventure", "the centred header"),
            (1, "N = next subject", "row 1's left label"),
            (1, "P = previous", "row 1's right label"),
            (2, "RETURN = read subject", "row 2's left label"),
            (2, "Q = resume game", "row 2's right label"),
        ] {
            assert!(
                rows[row].contains(text),
                "honor={honor} at {w}x{h}: {label} must read {text:?} — got {:?}. A blank run the \
                 game painted over this label's own whitespace mapped onto one of its GLYPH \
                 columns and erased it (SQ-0727); the blank past a label's pixel end reaches back \
                 inside its cell span and clips the tail the same way.",
                rows[row]
            );
        }
    }
}

#[test]
fn advent_help_bar_keeps_every_character_honoring_game_colours() {
    advent_help_bar_keeps_every_character(true);
}

#[test]
fn advent_help_bar_keeps_every_character_theme_only() {
    advent_help_bar_keeps_every_character(false);
}
