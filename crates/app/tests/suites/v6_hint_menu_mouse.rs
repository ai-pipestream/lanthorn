//! SQ-0951: a click on an InvisiClues topic selects THAT topic, at any pane width.
//!
//! The hint screen says "(Or use mouse.)", and it is ONE screen shared by two
//! games: SQ-0934 measured Zork Zero's and Shogun's hint frames against each
//! other and found the same ring artwork, the same four header strings at the
//! same native rows, and only the middle rect — the grid carrying the topic
//! list — differing. So both are driven here, and a defect that shows on one
//! must be looked for on the other.
//!
//! # Why this suite exists rather than another unit case
//!
//! `v6_mouse_zork0`'s packed-region cases are built from this screen's real
//! numbers and are SYNTHETIC: they invert a [`V6ClickMap`] the test wrote
//! itself. A map that inverts its own numbers perfectly and disagrees with what
//! the renderer DREW passes them, and that is exactly what shipped. Nothing
//! clicked a real topic and asserted the right one came back.
//!
//! So this drives the whole chain the player's mouse takes: boot the game, reach
//! the menu, render the pane, read the click map the frame recorded, and hand
//! `map_click`'s pixel to the VM through `set_mouse` + ZSCII 254 — the same two
//! calls `main.rs` makes on a left-button Down. The assertion is the game's own
//! answer: which topic is drawn in reverse video afterwards.
//!
//! # The three defects it pins, all from one user report
//!
//!   "mouse clicks in help menu not aligned horizontally with screen 190x60
//!    (need to click far to the left). when screen is small (50x60) clicking to
//!    the left of a selection misclicks several items lower, but clicking on a
//!    selection does work in that case."
//!
//! 1. **The columns.** A promoted story GRID is not drawn by the in-box run
//!    packing at all — `render_node` hands it to `draw_grid`, which CENTRES the
//!    game's screen in the pane. Zork Zero's grid is 58 columns in a 138-column
//!    viewport at 190x60, so every topic is drawn forty columns right of where
//!    the click map looked for it: clicking a topic mapped to native x=410
//!    against a topic list printed at x=87, and the player had to click far to
//!    the left. At 50x60 the viewport is NARROWER than the grid, nothing is
//!    centred, and clicking a topic worked — the user saw both halves.
//! 2. **The row phase.** The map inverted a row INDEX, `(y-1)/16`, so it returned
//!    the middle of a 16px grid slot. Zork Zero's box starts at native y=78 and
//!    prints at y=79, so its rows are 79..94 and the slot's middle (72) is in the
//!    row ABOVE: clicking GENERAL QUESTIONS selected THE JESTER at every width.
//!    Shogun's box (y=70, text at 71) happens to fall the other way, which is why
//!    a one-game case would have called this fixed.
//! 3. **Beside the region.** A packed region published its row mapping only
//!    inside its own columns, so a click one column left of a topic — the ring's
//!    flank, a tiled band that is not on the letterbox grid at all — fell to the
//!    proportional inverse and reported a native y from elsewhere on the screen:
//!    "several items lower", thirteen of them for a click beside GLACIER.
//!
//! Specimens, with the turn count each frame is reached at (a frame is a fixture):
//!
//! | fixture                   | release | to the menu                    |
//! |---------------------------|---------|--------------------------------|
//! | `zork0-r393-s890714.z6`   | 393     | `hint`, then `y` — 2 inputs    |
//! | `shogun-r322-s890706.z6`  | 322     | Enter past the splash, `hint`, `y` |
//!
//! Skip-if-missing (gitignored stories), and non-vacuous: a fixture that is
//! present but yielded no click fails rather than passing quietly.

use std::path::PathBuf;

use app::engine::{Engine, WinNode};
use app::graphics::PictSource;
use app::session::{GameSession, InputKind};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

fn stories_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stories")
}

/// The two pane widths the report names. 190x60 is the wide case (the grid is
/// centred, so the column origin matters); 50x60 is narrower than the game's own
/// screen (nothing is centred, and only the row defects show).
const PANES: &[(u16, u16)] = &[(190, 60), (50, 60)];

/// Boot a press to its hint menu, or `None` when the gitignored story is absent.
fn hint_menu(file: &str, release: u16) -> Option<GameSession> {
    let path = stories_dir().join(file);
    let bytes = std::fs::read(&path).ok()?;
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]),
        release,
        "{file} is not the pinned release"
    );
    let mut picts = PictSource::new(blorb::resolve_resource_blorb(&path).map(|(b, _)| b));
    let dims = picts.all_pict_dims();
    let std_window = picts.std_window();
    let mut s = GameSession::new_with_trace(bytes, true, false, None, false, dims, std_window, None, None)
        .expect("the press should load and boot without a ZError");
    s.set_pict_source(Some(picts));
    s.flush_boot_pictures();
    let _ = s.take_transcript();
    // Zork Zero asks for a LINE first; Shogun holds a title splash on a CHAR read.
    // Answer whatever is in the way rather than assuming either.
    for _ in 0..8 {
        match s.pending_input() {
            InputKind::Line => break,
            InputKind::Char => {
                let _ = s.submit_char(13);
            }
            InputKind::Event => {
                let _ = s.submit("");
            }
        }
    }
    s.submit("hint");
    let entered = s.submit_char(b'y');
    assert!(entered.fault.is_none(), "{file}: entering the hint menu faulted: {:?}", entered.fault);
    Some(s)
}

/// The promoted middle grid's own text runs — the topic list (SQ-0934).
fn topic_runs(s: &GameSession) -> Vec<app::engine::PxText> {
    let model = s.screen();
    let WinNode::Layered(items) = &model.root else { return Vec::new() };
    let layout = app::render::v6_layout::classify_windows(items, zvm::screen::V6Cell::DEFAULT);
    match layout.story.map(|st| &st.node) {
        Some(WinNode::Grid(g)) => g.px_texts.clone(),
        _ => Vec::new(),
    }
}

/// The topic the game is showing in reverse video — its answer to a click.
fn selected(s: &GameSession) -> Vec<String> {
    topic_runs(s)
        .iter()
        .filter(|t| t.style & 1 != 0 && !t.text.trim().is_empty())
        .map(|t| t.text.trim().to_string())
        .collect()
}

fn state(honor: bool) -> app::state::AppState {
    let mut st = app::state::AppState::default();
    st.colors = app::colors::ColorScheme::terminal_default();
    // A real kitty cell size: this screen's topic list is glyphs over a placed
    // ring, which is the arrangement the click map has to invert.
    st.game_picker = Some(app::render::graphics::kitty_picker(8, 16));
    st.config.v6_render = app::config::V6RenderMode::Hybrid;
    st.config.honor_game_colours = honor;
    st
}

/// Cell rows of a rendered pane, as chars — `find_cells` searches these rather
/// than a joined `String`, because the ring's kitty placeholders are four bytes
/// each and a byte offset is not a column.
fn cell_rows(buf: &Buffer, area: Rect) -> Vec<Vec<char>> {
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buf.cell((x, y)).map_or(' ', |c| c.symbol().chars().next().unwrap_or(' ')))
                .collect()
        })
        .collect()
}

/// The (column, row) a string is drawn at, in CELLS.
fn find_cells(rows: &[Vec<char>], needle: &str) -> Option<(u16, u16)> {
    let n: Vec<char> = needle.chars().collect();
    for (y, r) in rows.iter().enumerate() {
        if r.len() < n.len() {
            continue;
        }
        for x in 0..=(r.len() - n.len()) {
            if r[x..x + n.len()] == n[..] {
                return Some((x as u16, y as u16));
            }
        }
    }
    None
}

/// Specimens: press, release, and four topics spread down the list — the first
/// (which the game selects by default, so it also proves a click can be a no-op
/// rather than a jump) and three further down, where a row-phase or a row-scale
/// error has room to show.
const SPECIMENS: &[(&str, u16, &[&str])] = &[
    ("zork0-r393-s890714.z6", 393, &["PROLOGUE", "GLACIER", "THE JESTER", "GENERAL QUESTIONS"]),
    ("shogun-r322-s890706.z6", 322, &["Erasmus", "Mariko", "Seppuku", "Ninja"]),
];

fn a_click_on_a_topic_selects_that_topic(honor: bool) {
    let mut any_present = false;
    let mut clicks = 0usize;
    for (file, release, topics) in SPECIMENS {
        if !stories_dir().join(file).exists() {
            eprintln!("SKIP: gitignored story missing at {}", stories_dir().join(file).display());
            continue;
        }
        any_present = true;
        let s = hint_menu(file, *release).expect("present fixture must boot");

        // Premise, so a frame that stopped being this screen cannot pass vacuously.
        let runs = topic_runs(&s);
        assert!(
            runs.len() >= 15,
            "{file}: the hint menu's topic list is the promoted story GRID (SQ-0934); got {} runs",
            runs.len()
        );

        for &(w, h) in PANES {
            let st = state(honor);
            let area = Rect::new(0, 0, w, h);
            let mut buf = Buffer::empty(area);
            let model = s.screen();
            let _ = app::render::screen::render_story_pane(&model, false, None, &st, area, &mut buf);
            let rows = cell_rows(&buf, area);
            let map = st
                .graphics_render
                .borrow()
                .last_v6_map
                .clone()
                .unwrap_or_else(|| panic!("{file} {w}x{h}: a v6 frame records a click map"));

            // Shape guards. This screen is a ring, its topic list is drawn with
            // GLYPHS, and — at the wide pane — the grid is centred well right of
            // the viewport's left edge, which is the condition that made the
            // column defect visible at all. Without the last one a 190x60 case
            // would assert nothing this suite is about.
            assert_eq!(
                st.v6_path_log.borrow().last().map(|(l, _)| l.clone()),
                Some("hybrid-ring".into()),
                "{file} {w}x{h}: the hint screen takes the ring"
            );
            let first = topics[0];
            let (col, _) = find_cells(&rows, first).unwrap_or_else(|| {
                panic!("{file} {w}x{h}: {first:?} must be drawn as glyphs on this frame")
            });
            let packed_left = map
                .packed_text
                .iter()
                .find_map(|p| p.cols.map(|(l, _, _)| l))
                .unwrap_or_else(|| panic!("{file} {w}x{h}: the story box publishes a column mapping"));
            assert_eq!(
                packed_left, col,
                "{file} {w}x{h}: the click map's first packed column must BE the column the \
                 topic list is drawn on — the two disagreeing by the grid's centring is SQ-0951"
            );
            if w == 190 {
                assert!(
                    col >= 40,
                    "{file} {w}x{h}: non-vacuity — a wide pane centres the game's own {} columns \
                     far right of the viewport's left edge, and {first:?} at column {col} means \
                     this case is not exercising that",
                    map.packed_text.iter().filter_map(|p| p.cols.map(|(_, n, _)| n)).max().unwrap_or(0)
                );
            }

            for topic in topics.iter() {
                let (col, row) = find_cells(&rows, topic)
                    .unwrap_or_else(|| panic!("{file} {w}x{h}: {topic:?} is not drawn"));
                let run = runs
                    .iter()
                    .find(|t| t.text.trim() == *topic)
                    .unwrap_or_else(|| panic!("{file}: {topic:?} is not a topic run"));

                // (a) The geometry: the cell the player sees the topic on must map
                // into the topic's own character cell — its first 8x16 cell, at the
                // native x and y the game printed it at.
                let (gx, gy) = map
                    .map_click(col, row)
                    .unwrap_or_else(|| panic!("{file} {w}x{h}: {topic:?}'s own cell must map"));
                assert!(
                    (run.x..run.x + 8).contains(&gx),
                    "{file} {w}x{h}: clicking {topic:?} where it is drawn (cell {col},{row}) maps to \
                     native x={gx}, but the game printed it at x={} — the player must not have to \
                     click somewhere else to press it",
                    run.x
                );
                assert!(
                    (run.y..run.y + 16).contains(&gy),
                    "{file} {w}x{h}: clicking {topic:?} maps to native y={gy}, outside the row the \
                     game printed it on ({}..{})",
                    run.y,
                    run.y + 16
                );

                // (b) The game's own answer, through the two calls main.rs makes.
                // A fresh session per click: each one takes the menu somewhere.
                let mut victim = hint_menu(file, *release).expect("present fixture must boot");
                victim.set_mouse(gy, gx);
                let r = victim.submit_char(254);
                assert!(r.fault.is_none(), "{file} {w}x{h}: the click faulted: {:?}", r.fault);
                assert_eq!(
                    selected(&victim),
                    vec![topic.to_string()],
                    "{file} {w}x{h}: clicking {topic:?} where it is drawn must select it"
                );
                clicks += 1;

                // (c) …and a click just LEFT of it, which the user reported landing
                // several items lower, must not select a DIFFERENT topic. That cell
                // is the frame's own artwork, so its column stays proportional — but
                // the row under the pointer is the row under the pointer.
                let left = col.saturating_sub(4);
                if let Some((lx, ly)) = map.map_click(left, row) {
                    assert!(
                        (run.y..run.y + 16).contains(&ly),
                        "{file} {w}x{h}: a click four columns left of {topic:?} maps to native \
                         y={ly}, off its row ({}..{}) — this is the \"misclicks several items \
                         lower\" half of SQ-0951",
                        run.y,
                        run.y + 16
                    );
                    let mut victim = hint_menu(file, *release).expect("present fixture must boot");
                    victim.set_mouse(ly, lx);
                    let r = victim.submit_char(254);
                    assert!(r.fault.is_none(), "{file} {w}x{h}: the click faulted: {:?}", r.fault);
                    let sel = selected(&victim);
                    assert!(
                        sel.is_empty() || sel == vec![topic.to_string()],
                        "{file} {w}x{h}: clicking beside {topic:?} selected {sel:?}"
                    );
                }
            }
        }
    }
    assert!(
        !any_present || clicks > 0,
        "a present fixture yielded no click at all — the case passed without testing anything"
    );
}

#[test]
fn a_click_on_a_topic_selects_that_topic_honoring_game_colours() {
    a_click_on_a_topic_selects_that_topic(true);
}

#[test]
fn a_click_on_a_topic_selects_that_topic_theme_only() {
    a_click_on_a_topic_selects_that_topic(false);
}
