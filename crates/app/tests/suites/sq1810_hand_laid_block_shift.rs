//! SQ-1810: lower-window text a width-caching story laid out for 80 columns by hand.
//!
//! A v4+ story that caches the screen width is told 80 columns whatever the pane is
//! (SQ-1809). Some of them centre or indent lower-window text for 80 with leading
//! spaces — AMFV's opening quotation is indented 22, 23 and 39 columns — and in a
//! 45-column pane that text broke up: the long line wrapped and the attribution
//! landed on a row of its own. `render::hand_layout` shifts such a block left, at
//! RENDER time only, by the amount its furthest edge overflows the pane (capped by
//! the block's smallest indent).
//!
//! Real-story cases skip vacuously without `stories/`. Each names its release and the
//! turn count it was measured at, and guards the shape of the frame (a non-vacuity
//! assertion) before measuring.

use app::clipboard::{Point, Selection};
use app::engine::Engine;
use app::machine_boot::MachineBoot;
use app::session::{GameSession, InputKind};
use app::state::{AppState, TranscriptKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::fixture_paths::fixture_path;

/// Boot `file` told a `pane_cols`-column pane (a caching story is told 80 anyway).
fn boot(file: &str, pane_cols: u16) -> Option<GameSession> {
    let path = fixture_path(file);
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("SKIP: story missing at {}", path.display());
        return None;
    };
    let boot = MachineBoot::bare();
    Some(
        GameSession::new_for_machine(bytes, true, false, false, Vec::new(), Some((25, pane_cols)), None, &boot)
            .unwrap_or_else(|e| panic!("{file} boots: {e:?}")),
    )
}

/// Fold what the session has printed so far into the transcript.
fn flush(s: &mut GameSession, state: &mut AppState) {
    let t = s.take_transcript();
    if !t.is_empty() {
        state.push_transcript_runs(&t, TranscriptKind::Story, &[]);
    }
}

/// Press Enter at keypress prompts, then type `cmds` at line prompts.
fn drive(s: &mut GameSession, state: &mut AppState, cmds: &[&str]) {
    flush(s, state);
    let mut cmds = cmds.iter();
    for _ in 0..12 {
        let r = match s.pending_input() {
            InputKind::Char => s.submit_char(13),
            InputKind::Line => match cmds.next() {
                Some(c) => s.submit(c),
                None => break,
            },
            InputKind::Event => break,
        };
        state.push_transcript_runs(&r.transcript, TranscriptKind::Story, &[]);
    }
}

/// Render the story pane `w` columns wide (the told width is published first, as the
/// run loop's per-frame poll does) and return every row as text. The transcript body
/// is one column narrower than the pane (the scrollbar's column), so a pane of `w + 1`
/// wraps at `w`: the cases below say "46-column pane" for a 45-column wrap width.
fn frame(s: &mut GameSession, state: &AppState, w: u16) -> Vec<String> {
    frame_h(s, state, w, 25)
}

/// [`frame`] with a chosen pane height, for text that scrolls off a 25-row pane.
fn frame_h(s: &mut GameSession, state: &AppState, w: u16, h: u16) -> Vec<String> {
    app::host::screen::sync_zvm_screen_dims(s, state, (w, h));
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    app::render::screen::render_story_pane(&s.screen(), false, None, state, area, &mut buf);
    (0..area.height)
        .map(|y| (0..area.width).map(|x| buf.cell((x, y)).unwrap().symbol().chars().next().unwrap_or(' ')).collect())
        .collect()
}

fn indent(row: &str) -> usize {
    row.chars().take_while(|c| *c == ' ').count()
}

fn row_of(rows: &[String], needle: &str) -> usize {
    rows.iter().position(|r| r.contains(needle)).unwrap_or_else(|| panic!("{needle:?} is on screen:\n{}", rows.join("\n")))
}

const AMFV: &str = "amfv-r77-s850814.z4";

/// AMFV r77 (serial 850814), 0 turns in: the boot prints the opening quotation and
/// waits at "[Hit any key to continue.]". Told 80 in a 45-column pane.
fn amfv_at_the_quotation() -> Option<(GameSession, AppState)> {
    let mut s = boot(AMFV, 45)?;
    let mut state = AppState::default();
    flush(&mut s, &mut state);
    // Non-vacuity: the story is told 80, and the quotation is in the transcript as
    // the game printed it, 22/23/39 columns in.
    assert_eq!(s.fixed_cols(), Some(80), "AMFV r77 caches the width");
    let at = |needle: &str| state.transcript.iter().find(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle:?} printed")).clone();
    assert_eq!(indent(&at("Tomorrow never yet")), 22);
    assert_eq!(indent(&at("On any human being")), 23);
    assert_eq!(indent(&at("William Marsden")), 39);
    Some((s, state))
}

#[test]
fn amfv_r77_quotation_keeps_its_shape_in_a_45_column_pane() {
    let Some((mut s, state)) = amfv_at_the_quotation() else { return };
    let rows = frame(&mut s, &state, 46);
    assert_eq!(state.transcript_geom.get().unwrap().area.width, 45, "the wrap width");
    let y = row_of(&rows, "Tomorrow never yet");
    // Edge 57 against a 45-column pane: shift 12, so 22/23/39 become 10/11/27.
    assert_eq!(indent(&rows[y]), 10, "{:?}", rows[y]);
    assert_eq!(indent(&rows[y + 1]), 11, "{:?}", rows[y + 1]);
    assert_eq!(indent(&rows[y + 2]), 27, "{:?}", rows[y + 2]);
    assert!(rows[y + 1].contains("On any human being rose or set."), "no wrap: {:?}", rows[y + 1]);
    assert!(rows[y + 2].trim() == "-- William Marsden", "the attribution is not alone on a wrapped row: {:?}", rows[y + 2]);
    assert!(rows[y + 2].trim_end().chars().count() <= 45);
}

#[test]
fn amfv_r77_quotation_returns_to_its_own_indents_at_80_and_120_and_back() {
    let Some((mut s, state)) = amfv_at_the_quotation() else { return };
    // Panes one wider than the wrap width they give (see `frame`).
    for (pane, wrap, want) in [
        (46u16, 45u16, [10usize, 11, 27]),
        (81, 80, [22, 23, 39]),
        (121, 120, [22, 23, 39]),
        (46, 45, [10, 11, 27]),
        (61, 60, [22, 23, 39]), // edge 57 fits: nothing moves
        (41, 40, [5, 6, 22]),
    ] {
        let rows = frame(&mut s, &state, pane);
        assert_eq!(state.transcript_geom.get().unwrap().area.width, wrap);
        let y = row_of(&rows, "Tomorrow never yet");
        assert_eq!([indent(&rows[y]), indent(&rows[y + 1]), indent(&rows[y + 2])], want, "wrap width {wrap}");
    }
    // The stored transcript never changed.
    assert!(state.transcript.iter().any(|l| l.starts_with(&" ".repeat(39)) && l.contains("William Marsden")));
}

/// What copy takes: the rows as DRAWN (the shifted text), because `clipboard::extract`
/// reads the wrapped rows. Reported, not changed.
#[test]
fn copy_over_a_shifted_block_takes_what_is_drawn() {
    let Some((mut s, mut state)) = amfv_at_the_quotation() else { return };
    let rows = frame(&mut s, &state, 46);
    let y = row_of(&rows, "Tomorrow never yet");
    let geom = state.transcript_geom.get().expect("geometry published");
    let abs = geom.first_abs_row + y - geom.area.y as usize;
    state.selection = Some(Selection { anchor: Point { row: abs, col: 0 }, head: Point { row: abs + 2, col: 44 } });
    let _ = frame(&mut s, &state, 46);
    let copied = state.selection_text.borrow().clone().expect("a selection copies");
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), 3, "{copied:?}");
    assert_eq!(indent(lines[0]), 10);
    assert_eq!(indent(lines[2]), 27);
}

/// Planetfall r10 (Solid Gold, serial 880531), the opening Enter then `kill me` (1 Enter,
/// 1 command): the death banner is indented 4 and 25 long, so its edge is column 29 and
/// it passes a 27-column wrap width by two.
///
/// This replaces the inventory case: that list was indented 2 and 4, and the 2-space
/// items are no longer hand-laid (SQ-1810 follow-up: a block needs 4+ spaces). Before
/// the change the inventory block moved by its smallest indent, 2; now the 2-space
/// items never join a block and keep their indent, and the lone 4-space nested line
/// fits, so the inventory is not shifted at all.
#[test]
fn planetfall_r10_death_banner_shifts_in_a_27_column_pane() {
    let Some(mut s) = boot("planetfall-invclues-r10-s880531.z5", 27) else { return };
    let mut state = AppState::default();
    drive(&mut s, &mut state, &["kill me"]);
    assert_eq!(s.fixed_cols(), Some(80), "Planetfall caches the width");
    // Non-vacuity: the banner is there, indented 4 as the game printed it, and its
    // edge (29) passes the 27-column wrap width but not the told 80.
    let src = state.transcript.iter().find(|l| l.contains("You have died")).expect("the death banner is printed").clone();
    assert_eq!(src, "    ****  You have died  ****");

    let wide = frame_h(&mut s, &state, 81, 120);
    assert_eq!(indent(&wide[row_of(&wide, "You have died")]), 4);

    let rows = frame_h(&mut s, &state, 28, 120);
    let y = row_of(&rows, "You have died");
    assert_eq!(indent(&rows[y]), 2, "overflow 2, smallest indent 4: moves 2 left: {:?}", rows[y]);
    assert!(rows[y].contains("****  You have died  ****"), "the banner stays on one row: {:?}", rows[y]);
}

/// Leather Goddesses of Phobos r4 (serial 880405), boot, the opening Enter, then `look` (1 Enter,
/// 1 command): its paragraphs are indented 3, and `   You feel an urge.` is short
/// enough to be a one-line "block" (edge 20) that a 15-column wrap width overflows.
/// A 3-space paragraph is not hand-laid, so it keeps its indent.
#[test]
fn lgop_r4_three_space_paragraph_keeps_its_indent_in_a_narrow_pane() {
    let Some(mut s) = boot("leathergoddesses-invclues-r4-s880405.z5", 15) else { return };
    let mut state = AppState::default();
    drive(&mut s, &mut state, &["look"]);
    assert_eq!(s.fixed_cols(), Some(80), "LGOP caches the width");
    // Non-vacuity: the short paragraph is printed indented 3, and is wider than the pane.
    let src = state.transcript.iter().find(|l| l.contains("You feel an urge")).expect("the paragraph is printed").clone();
    assert_eq!(src, "   You feel an urge.");
    assert!(src.chars().count() > 15);

    let rows = frame(&mut s, &state, 16);
    let y = row_of(&rows, "You feel");
    assert_eq!(indent(&rows[y]), 3, "{:?}", rows[y]);
}

/// The wrap cache is incremental, and a line that arrives later can join a block
/// already wrapped and move its shift. The result must equal a from-scratch render.
#[test]
fn a_block_that_grows_across_frames_is_laid_out_whole() {
    let Some((mut s, _)) = amfv_at_the_quotation() else { return };
    let quote = [
        format!("{}\"Tomorrow never yet", " ".repeat(22)),
        format!("{}On any human being rose or set.", " ".repeat(23)),
        format!("{}-- William Marsden", " ".repeat(39)),
    ];
    let mut grown = AppState::default();
    grown.push_transcript_runs("Before.", TranscriptKind::Story, &[]);
    for (n, l) in quote.iter().enumerate() {
        grown.push_transcript_runs(l, TranscriptKind::Story, &[]);
        let _ = frame(&mut s, &grown, 46); // a frame after each line
        assert!(n < 3);
    }
    let mut whole = AppState::default();
    whole.push_transcript_runs("Before.", TranscriptKind::Story, &[]);
    for l in &quote {
        whole.push_transcript_runs(l, TranscriptKind::Story, &[]);
    }
    let (a, b) = (frame(&mut s, &grown, 46), frame(&mut s, &whole, 46));
    let y = row_of(&a, "Tomorrow never yet");
    assert_eq!([indent(&a[y]), indent(&a[y + 1]), indent(&a[y + 2])], [10, 11, 27], "{a:#?}");
    assert_eq!(a, b, "incremental and from-scratch renders agree");
}
